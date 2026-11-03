//! Anime4K as the screen filter, see ScreenFilter::Anime4k: Restore CNN M
//! when its chain runs it, then CNN x2 M, a shader a pass, each drawing one
//! tensor at the screen's size from those before it, SMAA's result the
//! first, and the last the screen twice as big. Restore's last pass is the
//! screen restored, which the upscaler reads as its picture.
//!
//! one set of images serves both frames in flight, as SMAA's do.

use ash::vk;

use super::fsr::{begin_pass, create_target_pass, picture_set, Pipeline, Target, COLOUR_FORMAT};
use super::{create_descriptor_layout, create_pipeline, fail, set_viewport, vk_fail};
use crate::anime4k::{Ending, Network};
use crate::backend::{Anime4kChain, PresentError, Viewport};

/// Anime4K's CNN x2 M, a shader a pass, made from its network by
/// anime4k.rs's write_screen_shaders test.
const UPSCALE_SPIRV: [&[u8]; 9] = [
    include_bytes!("../../../shaders/anime4k/pass0.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass1.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass2.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass3.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass4.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass5.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass6.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass7.frag.spv"),
    include_bytes!("../../../shaders/anime4k/pass8.frag.spv"),
];
/// Anime4K's Restore CNN M, run before the upscaler as Anime4K's fast mode
/// does, made the same way.
const RESTORE_SPIRV: [&[u8]; 8] = [
    include_bytes!("../../../shaders/anime4k/restore0.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore1.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore2.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore3.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore4.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore5.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore6.frag.spv"),
    include_bytes!("../../../shaders/anime4k/restore7.frag.spv"),
];
/// what Anime4K's passes write: signed, and finer than a colour.
const TENSOR_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// what a pass writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    /// a tensor at the screen's size, numbered from 1 across both networks,
    /// tensor 0 being SMAA's result.
    Tensor(usize),
    /// the screen twice as big, the upscaler's last.
    Doubled,
}

/// a pass of the graph: the tensors it reads, in the order its shader binds
/// them, what it writes, and its shader.
struct GraphPass {
    inputs: Vec<usize>,
    output: Output,
    spirv: &'static [u8],
}

/// every pass the filter runs, in order, and the format of each tensor from
/// 1 on: a colour for the screen restored, signed and finer for the rest.
struct Graph {
    passes: Vec<GraphPass>,
    tensor_formats: Vec<vk::Format>,
}

/// the passes of the upscaler's network, and Restore's before them when
/// there is a restorer, numbered as one graph.
fn graph(upscaler: &Network, restorer: Option<&Network>) -> Result<Graph, PresentError> {
    let (mut passes, mut tensor_formats) = (Vec::new(), Vec::new());
    // the picture the upscaler reads: SMAA's result, or Restore's of it
    let mut picture = 0;
    if let Some(restorer) = restorer {
        let &Ending::Restores(restored) = restorer.ending() else { return Err(fail("its Restore network does not restore")) };
        if restorer.graph().count() != RESTORE_SPIRV.len() {
            return Err(fail(format!("its Restore network has {} passes and {} shaders", restorer.graph().count(), RESTORE_SPIRV.len())));
        }
        for ((inputs, output), spirv) in restorer.graph().zip(RESTORE_SPIRV) {
            passes.push(GraphPass { inputs: inputs.to_vec(), output: Output::Tensor(output), spirv });
        }
        picture = restored;
        tensor_formats.extend((1..restorer.tensors()).map(|tensor| if tensor == picture { COLOUR_FORMAT } else { TENSOR_FORMAT }));
    }
    let Ending::DepthToSpace(residuals) = upscaler.ending() else { return Err(fail("its network does not double")) };
    // the upscaler's tensors come after Restore's, its picture before them
    let global = |tensor: usize| if tensor == 0 { picture } else { picture + tensor };
    if upscaler.graph().count() + 1 != UPSCALE_SPIRV.len() {
        return Err(fail(format!("its network has {} passes and {} shaders", upscaler.graph().count() + 1, UPSCALE_SPIRV.len())));
    }
    for ((inputs, output), spirv) in upscaler.graph().zip(UPSCALE_SPIRV) {
        passes.push(GraphPass { inputs: inputs.iter().map(|&tensor| global(tensor)).collect(), output: Output::Tensor(global(output)), spirv });
    }
    tensor_formats.extend((1..upscaler.tensors()).map(|_| TENSOR_FORMAT));
    // the last puts the residuals on the picture
    let inputs = std::iter::once(picture).chain(residuals.iter().map(|&tensor| global(tensor))).collect();
    passes.push(GraphPass { inputs, output: Output::Doubled, spirv: UPSCALE_SPIRV[UPSCALE_SPIRV.len() - 1] });
    Ok(Graph { passes, tensor_formats })
}

/// Anime4K's passes as the FSR filter holds them: made when its filter is
/// first wanted, let go of once another is chosen.
pub(super) enum Anime4kState {
    /// not made, as the filter is not Anime4K's or it was let go of.
    Unmade,
    Made(Box<Anime4k>),
    /// making them failed, which was said: the filter is FSR from then on.
    Failed,
}

impl Anime4kState {
    pub(super) fn made(&self) -> Option<&Anime4k> {
        match self {
            Anime4kState::Made(anime4k) => Some(anime4k),
            Anime4kState::Unmade | Anime4kState::Failed => None,
        }
    }

    /// makes for a frame what the chain draws with: its passes, made again
    /// when the chain changed, and the images of each screen that has its
    /// SMAA result's view and size in mains. it waits for the GPU at most
    /// once, before letting go of anything a frame in flight may read. a
    /// failure to make them is said, and the filter is FSR from then on.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        colour_pass: vk::RenderPass,
        picture_layout: vk::DescriptorSetLayout,
        sampler: vk::Sampler,
        chain: Anime4kChain,
        mains: [Option<(vk::ImageView, (u32, u32))>; 3],
    ) -> Result<(), PresentError> {
        let mut waited = false;
        if self.made().is_some_and(|anime4k| anime4k.chain != chain) {
            wait_once(device, &mut waited)?;
            self.destroy(device);
        }
        if let Anime4kState::Unmade = self {
            *self = match Anime4k::new(device, colour_pass, chain) {
                Ok(anime4k) => Anime4kState::Made(Box::new(anime4k)),
                Err(error) => {
                    log::warn!("Anime4K's screen filter could not be made, {error}, so it is FSR from now on");
                    Anime4kState::Failed
                }
            };
        }
        let Anime4kState::Made(anime4k) = &mut *self else { return Ok(()) };
        for (index, main) in mains.into_iter().enumerate() {
            // a screen's images go whenever SMAA's are made again
            let Some((main, size)) = main.filter(|_| anime4k.screens[index].is_none()) else { continue };
            if let Err(error) = anime4k.make_screen(device, memory_properties, picture_layout, sampler, index, main, size) {
                log::warn!("Anime4K's screen filter could not be made, {error}, so it is FSR from now on");
                wait_once(device, &mut waited)?;
                self.destroy(device);
                *self = Anime4kState::Failed;
                break;
            }
        }
        Ok(())
    }

    /// lets go of the passes and their images once the GPU is done with
    /// them; a failure stays one.
    pub(super) fn release(&mut self, device: &ash::Device) -> Result<(), PresentError> {
        if self.made().is_some() {
            wait_once(device, &mut false)?;
            self.destroy(device);
        }
        Ok(())
    }

    /// lets go of a screen's images, SMAA's being made again.
    pub(super) fn drop_screen(&mut self, device: &ash::Device, index: usize) {
        if let Anime4kState::Made(anime4k) = self {
            anime4k.drop_screen(device, index);
        }
    }

    /// lets go of the passes and their images, which nothing may still
    /// read; a failure stays one.
    pub(super) fn destroy(&mut self, device: &ash::Device) {
        if self.made().is_some() {
            if let Anime4kState::Made(anime4k) = std::mem::replace(self, Anime4kState::Unmade) {
                anime4k.destroy(device);
            }
        }
    }
}

/// waits for the GPU to be idle, unless it was waited for already.
fn wait_once(device: &ash::Device, waited: &mut bool) -> Result<(), PresentError> {
    if !std::mem::replace(waited, true) {
        unsafe { device.device_wait_idle() }.map_err(vk_fail("waiting for idle"))?;
    }
    Ok(())
}

/// Anime4K's passes for a chain, with their images for each screen. its
/// sets come from a pool of its own, with room for every screen's.
pub(super) struct Anime4k {
    chain: Anime4kChain,
    tensor_pass: vk::RenderPass,
    /// the FSR filter's, for the screen restored and doubled, not its own
    /// to destroy.
    colour_pass: vk::RenderPass,
    pool: vk::DescriptorPool,
    passes: Vec<Anime4kPass>,
    /// the format of each tensor from 1 on, tensor 0 being SMAA's result.
    tensor_formats: Vec<vk::Format>,
    /// the images of the top screen, the bottom one and the right eye's,
    /// made after SMAA's and let go of whenever those are.
    screens: [Option<Anime4kScreen>; 3],
}

/// one of Anime4K's passes, made.
struct Anime4kPass {
    pipeline: Pipeline,
    set_layout: vk::DescriptorSetLayout,
    /// the tensors it reads, in the order its shader binds them.
    inputs: Vec<usize>,
    output: Output,
}

/// Anime4K's images for one screen at its size, with the sets reading them.
struct Anime4kScreen {
    /// tensors 1 on, at the screen's size, tensor 0 being SMAA's result.
    tensors: Vec<Target>,
    /// each pass's set, reading its tensors.
    sets: Vec<vk::DescriptorSet>,
    /// the screen twice as big, and the set EASU reads it through.
    result: Target,
    result_input: vk::DescriptorSet,
    size: (u32, u32),
}

impl Anime4k {
    /// its passes for the chain, drawing the screen restored and doubled
    /// in the FSR filter's colour_pass.
    fn new(device: &ash::Device, colour_pass: vk::RenderPass, chain: Anime4kChain) -> Result<Anime4k, PresentError> {
        let upscaler = crate::anime4k::screen_network().ok_or_else(|| fail("its network could not be read"))?;
        let restorer = match chain {
            Anime4kChain::Upscale => None,
            Anime4kChain::RestoreThenUpscale => Some(crate::anime4k::restore_network().ok_or_else(|| fail("its Restore network could not be read"))?),
        };
        let graph = graph(upscaler, restorer)?;
        let tensor_pass = create_target_pass(device, TENSOR_FORMAT)?;
        let mut anime4k = Anime4k {
            chain,
            tensor_pass,
            colour_pass,
            pool: vk::DescriptorPool::null(),
            passes: Vec::new(),
            tensor_formats: graph.tensor_formats,
            screens: [None, None, None],
        };
        match anime4k.make_passes(device, &graph.passes) {
            Ok(()) => Ok(anime4k),
            Err(error) => {
                anime4k.destroy(device);
                Err(error)
            }
        }
    }

    /// the pool, and a pipeline and a set layout a pass, kept as they are
    /// made, for destroy to let go of should one fail.
    fn make_passes(&mut self, device: &ash::Device, graph: &[GraphPass]) -> Result<(), PresentError> {
        let pictures = graph.iter().map(|pass| pass.inputs.len() as u32).sum::<u32>() + 1;
        let sizes = [vk::DescriptorPoolSize::default().ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(3 * pictures)];
        let info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .pool_sizes(&sizes)
            .max_sets(3 * (graph.len() as u32 + 1));
        self.pool = unsafe { device.create_descriptor_pool(&info, None) }.map_err(vk_fail("creating Anime4K's descriptor pool"))?;
        for pass in graph {
            let set_layout = create_descriptor_layout(device, pass.inputs.len() as u32)?;
            match create_pipeline(device, self.pass_for(pass.output), &[set_layout], pass.spirv, 4) {
                Ok(pipeline) => self.passes.push(Anime4kPass { pipeline, set_layout, inputs: pass.inputs.clone(), output: pass.output }),
                Err(error) => {
                    unsafe { device.destroy_descriptor_set_layout(set_layout, None) };
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// the render pass a pass writing output draws in.
    fn pass_for(&self, output: Output) -> vk::RenderPass {
        match output {
            Output::Doubled => self.colour_pass,
            Output::Tensor(tensor) if self.tensor_formats[tensor - 1] == COLOUR_FORMAT => self.colour_pass,
            Output::Tensor(_) => self.tensor_pass,
        }
    }

    /// makes its images for a screen of this size, whose SMAA result is
    /// main.
    #[allow(clippy::too_many_arguments)]
    fn make_screen(
        &mut self,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        picture_layout: vk::DescriptorSetLayout,
        sampler: vk::Sampler,
        index: usize,
        main: vk::ImageView,
        size: (u32, u32),
    ) -> Result<(), PresentError> {
        let (mut tensors, mut sets) = (Vec::new(), Vec::new());
        // what was made goes again, should anything fail
        let made = (|| {
            for (tensor, &format) in self.tensor_formats.iter().enumerate() {
                tensors.push(Target::new(device, memory_properties, self.pass_for(Output::Tensor(tensor + 1)), format, size)?);
            }
            for pass in &self.passes {
                let views: Vec<vk::ImageView> = pass.inputs.iter().map(|&tensor| if tensor == 0 { main } else { tensors[tensor - 1].texture.view }).collect();
                sets.push(picture_set(device, self.pool, pass.set_layout, sampler, &views)?);
            }
            let result = Target::new(device, memory_properties, self.colour_pass, COLOUR_FORMAT, (size.0 * 2, size.1 * 2))?;
            match picture_set(device, self.pool, picture_layout, sampler, &[result.texture.view]) {
                Ok(set) => Ok((result, set)),
                Err(error) => {
                    result.destroy(device);
                    Err(error)
                }
            }
        })();
        match made {
            Ok((result, result_input)) => {
                self.screens[index] = Some(Anime4kScreen { tensors, sets, result, result_input, size });
                Ok(())
            }
            Err(error) => {
                if !sets.is_empty() {
                    unsafe {
                        let _ = device.free_descriptor_sets(self.pool, &sets);
                    }
                }
                tensors.iter().for_each(|tensor| tensor.destroy(device));
                Err(error)
            }
        }
    }

    /// the set EASU reads a screen twice as big through, once its images
    /// are made.
    pub(super) fn doubled(&self, index: usize) -> Option<vk::DescriptorSet> {
        self.screens[index].as_ref().map(|screen| screen.result_input)
    }

    /// records its passes over a screen, after SMAA's, once its images are
    /// made.
    pub(super) fn record(&self, device: &ash::Device, command_buffer: vk::CommandBuffer, index: usize) {
        let Some(screen) = &self.screens[index] else { return };
        for (pass, &set) in self.passes.iter().zip(&screen.sets) {
            let (target, (width, height)) = match pass.output {
                Output::Doubled => (&screen.result, (screen.size.0 * 2, screen.size.1 * 2)),
                Output::Tensor(tensor) => (&screen.tensors[tensor - 1], screen.size),
            };
            let extent = vk::Extent2D { width, height };
            let (layout, pipeline) = pass.pipeline;
            begin_pass(device, command_buffer, self.pass_for(pass.output), target.framebuffer, extent);
            set_viewport(device, command_buffer, Viewport { x: 0.0, y: 0.0, width: width as f32, height: height as f32 }, extent);
            unsafe {
                device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
                device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[set], &[]);
                device.cmd_draw(command_buffer, 3, 1, 0, 0);
                device.cmd_end_render_pass(command_buffer);
            }
        }
    }

    /// lets go of a screen's images.
    fn drop_screen(&mut self, device: &ash::Device, index: usize) {
        let Some(screen) = self.screens[index].take() else { return };
        let mut sets = screen.sets;
        sets.push(screen.result_input);
        unsafe {
            let _ = device.free_descriptor_sets(self.pool, &sets);
        }
        screen.tensors.iter().for_each(|tensor| tensor.destroy(device));
        screen.result.destroy(device);
    }

    /// lets go of every screen's images, the passes, the pool and its own
    /// render pass, leaving the FSR filter's colour pass to it.
    fn destroy(mut self, device: &ash::Device) {
        for index in 0..3 {
            self.drop_screen(device, index);
        }
        unsafe {
            for pass in self.passes.drain(..) {
                let (layout, pipeline) = pass.pipeline;
                device.destroy_pipeline(pipeline, None);
                device.destroy_pipeline_layout(layout, None);
                device.destroy_descriptor_set_layout(pass.set_layout, None);
            }
            if self.pool != vk::DescriptorPool::null() {
                device.destroy_descriptor_pool(self.pool, None);
            }
            device.destroy_render_pass(self.tensor_pass, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fsr::testing::TestGpu;
    use super::*;

    /// every tensor a pass reads is the picture or one an earlier pass
    /// wrote, and each is written once.
    fn check_order(graph: &Graph) {
        let mut written = vec![false; graph.tensor_formats.len() + 1];
        written[0] = true;
        for pass in &graph.passes {
            assert!(pass.inputs.iter().all(|&tensor| written[tensor]), "{:?} read before it is written", pass.inputs);
            if let Output::Tensor(tensor) = pass.output {
                assert!(!std::mem::replace(&mut written[tensor], true), "tensor {tensor} written twice");
            }
        }
        assert!(written.iter().all(|&written| written), "a tensor no pass writes");
    }

    /// the upscaler alone: a pass a shader, the picture read first, the
    /// screen doubled last from the picture and the residuals.
    #[test]
    fn the_upscalers_graph_doubles_the_picture() {
        let graph = graph(crate::anime4k::screen_network().unwrap(), None).unwrap();
        assert_eq!(graph.passes.len(), UPSCALE_SPIRV.len());
        assert_eq!(graph.passes[0].inputs, [0]);
        let last = graph.passes.last().unwrap();
        assert_eq!((last.output, last.inputs[0]), (Output::Doubled, 0));
        assert!(graph.tensor_formats.iter().all(|&format| format == TENSOR_FORMAT));
        check_order(&graph);
    }

    /// Restore's passes come first, its restored screen a colour the
    /// upscaler reads as its picture, first and last.
    #[test]
    fn restore_runs_before_the_upscaler_on_its_picture() {
        let graph = graph(crate::anime4k::screen_network().unwrap(), Some(crate::anime4k::restore_network().unwrap())).unwrap();
        assert_eq!(graph.passes.len(), RESTORE_SPIRV.len() + UPSCALE_SPIRV.len());
        let Output::Tensor(restored) = graph.passes[RESTORE_SPIRV.len() - 1].output else { panic!("Restore's last writes a tensor") };
        assert!(graph.tensor_formats[restored - 1] == COLOUR_FORMAT, "the screen restored is a colour");
        assert_eq!(graph.tensor_formats.iter().filter(|&&format| format == COLOUR_FORMAT).count(), 1);
        assert_eq!(graph.passes[RESTORE_SPIRV.len()].inputs, [restored], "the upscaler's first reads the screen restored");
        assert_eq!(graph.passes.last().unwrap().inputs[0], restored, "and its last adds to it");
        check_order(&graph);
    }

    /// on the GPU, when there is one: Anime4K's passes make a screen twice
    /// as big as its network does on the CPU, but for half floats' rounding.
    #[test]
    fn anime4k_on_the_gpu_draws_what_its_network_does_on_the_cpu() {
        let Some(gpu) = TestGpu::new() else { return };
        let (device, memory_properties) = (gpu.device(), &gpu.memory_properties);
        let colour_pass = create_target_pass(device, COLOUR_FORMAT).unwrap();

        // a red disc over a gradient, edges and slopes both
        let (width, height) = (32u32, 24u32);
        let texels: Vec<[u8; 4]> = (0..width * height)
            .map(|i| {
                let (x, y) = (i % width, i / width);
                let disc = (x as i32 - 14).pow(2) + (y as i32 - 11).pow(2) < 64;
                if disc { [230, 60, 40, 255] } else { [(x * 6) as u8, (y * 9) as u8, 120, 255] }
            })
            .collect();
        let pixels: Vec<u8> = texels.iter().flatten().copied().collect();
        let picture = gpu.upload(&pixels, (width, height));

        // the upscaler alone, then Restore before it
        for chain in [Anime4kChain::Upscale, Anime4kChain::RestoreThenUpscale] {
            let mut anime4k = Anime4k::new(device, colour_pass, chain).unwrap();
            anime4k.make_screen(device, memory_properties, gpu.picture_layout, gpu.sampler, 0, picture.view, (width, height)).unwrap();
            gpu.run(|command_buffer| anime4k.record(device, command_buffer, 0));
            let doubled = anime4k.screens[0].as_ref().unwrap().result.texture.image;
            let on_gpu = gpu.read_back(doubled, (width * 2, height * 2));
            // on the CPU: Restore's picture kept as a colour, as on the GPU, then doubled
            let source = match chain {
                Anime4kChain::RestoreThenUpscale => {
                    crate::anime4k::reference::restore(crate::anime4k::restore_network().unwrap(), &texels, width, height)
                }
                Anime4kChain::Upscale => texels.clone(),
            };
            let on_cpu = crate::anime4k::upscale(crate::anime4k::screen_network().unwrap(), &source, width, height);
            let worst = on_gpu.iter().zip(&on_cpu).map(|(gpu, cpu)| gpu.abs_diff(*cpu)).max().unwrap();
            assert!(worst <= 4, "{chain:?}: a channel {worst} off the CPU's");
            // and the networks did more than filter
            let filtered = crate::anime4k::reference::upscale_filtered(&texels, width, height);
            assert!(on_gpu.iter().zip(&filtered).any(|(gpu, filtered)| gpu.abs_diff(*filtered) > 8), "{chain:?}: no different from filtering");
            anime4k.destroy(device);
        }
        picture.destroy(device);
        unsafe { device.destroy_render_pass(colour_pass, None) };
    }
}
