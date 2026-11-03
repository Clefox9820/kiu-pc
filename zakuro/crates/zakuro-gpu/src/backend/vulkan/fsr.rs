//! the FSR filter's antialiasing and sharpening: SMAA 1x, the present
//! shader's EASU into an image the window's size, then RCAS onto the
//! window, with Anime4K's passes between SMAA and EASU when it is the
//! filter, see super::anime4k.

use ash::vk;

use super::anime4k::Anime4kState;
use super::{
    create_descriptor_layout, create_pipeline, fail, find_memory_type, set_viewport, transition, vk_fail, VulkanPresenter, CONSTANTS,
    FRAGMENT_SPIRV, WHOLE,
};
use crate::backend::{PresentError, ScreenFilter, Shown, Viewport};

const SMAA_EDGES_SPIRV: &[u8] = include_bytes!("../../../shaders/smaa_edges.frag.spv");
const SMAA_WEIGHTS_SPIRV: &[u8] = include_bytes!("../../../shaders/smaa_weights.frag.spv");
const SMAA_BLEND_SPIRV: &[u8] = include_bytes!("../../../shaders/smaa_blend.frag.spv");
const RCAS_SPIRV: &[u8] = include_bytes!("../../../shaders/rcas.frag.spv");

/// SMAA's precomputed tables, from its Textures/AreaTex.h and SearchTex.h:
/// the area each pattern of edges covers, two channels, and how far a
/// search's last step goes, one channel.
const SMAA_AREA: &[u8] = include_bytes!("../../../shaders/smaa_area.lut");
const SMAA_AREA_SIZE: (u32, u32) = (160, 560);
const SMAA_SEARCH: &[u8] = include_bytes!("../../../shaders/smaa_search.lut");
const SMAA_SEARCH_SIZE: (u32, u32) = (64, 16);

/// the bytes of push constants the SMAA passes read: where the screen lies
/// in its image and the bounds its samples keep within, as for the present
/// shader, then SMAA's metrics of the screen.
const SMAA_CONSTANTS: u32 = 48;
/// RCAS's: the first and last window pixels a screen covers, then the
/// sharpness.
const RCAS_CONSTANTS: u32 = 20;
/// how far RCAS sharpens below its most, in stops, each halving it, as
/// FsrRcasCon takes it: 0 the sharpest.
const RCAS_SHARPNESS_STOPS: f32 = 0.5;
/// the images the FSR filter draws in between, but for SMAA's edges.
pub(super) const COLOUR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const EDGES_FORMAT: vk::Format = vk::Format::R8G8_UNORM;

/// the descriptor sets the FSR filter takes from the presenter's pool:
/// SMAA's three for each of the three screens, made after the old ones go,
/// and the upscaled screens' one.
pub(super) const FSR_SETS: u32 = 3 * 3 + 1;
/// the pictures those sets hold: five for each screen's SMAA (its edges and
/// the two tables, its weights, its result), and the upscaled screens' one.
pub(super) const FSR_PICTURES: u32 = 3 * 5 + 1;

/// where a screen goes in the window, which picture it shows, and whether
/// the FSR filter antialiases and sharpens it there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Place {
    pub(super) viewport: Viewport,
    pub(super) shown: Shown,
    pub(super) through: Through,
}

/// how a place's screen goes onto the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Through {
    /// as it is, through the present shader's filter
    Plain,
    /// through SMAA, then drawn as it is
    Smaa,
    /// through SMAA, EASU into the window's size, then RCAS
    Sharpened,
}

/// how the FSR filter, when `fsr` it is on and works, draws a screen of
/// `size` pixels into `viewport`: antialiased at any size, upscaled and
/// sharpened as well when it is drawn bigger. the top screen's two pictures
/// in one for 3D keep to the shader's EASU alone.
fn through(fsr: bool, shown: Shown, viewport: Viewport, size: (u32, u32)) -> Through {
    if !fsr || shown == Shown::Both || size.0 == 0 || size.1 == 0 {
        Through::Plain
    } else if magnified(viewport, size) {
        Through::Sharpened
    } else {
        Through::Smaa
    }
}

/// a pipeline and its layout.
pub(super) type Pipeline = (vk::PipelineLayout, vk::Pipeline);

const NO_PIPELINE: Pipeline = (vk::PipelineLayout::null(), vk::Pipeline::null());

/// the screen a place shows: the top one, also for its two pictures in one,
/// the bottom one, or the top one's picture for the right eye.
pub(super) fn screen_index(shown: Shown) -> usize {
    match shown {
        Shown::Top | Shown::Both => 0,
        Shown::Bottom => 1,
        Shown::Right => 2,
    }
}

/// which screens a sharpened place shows: the top one, the bottom one and
/// the right eye's.
fn sharpened_screens(places: &[Place]) -> [bool; 3] {
    let mut sharpened = [false; 3];
    for place in places.iter().filter(|place| place.through == Through::Sharpened) {
        sharpened[screen_index(place.shown)] = true;
    }
    sharpened
}

/// whether a screen of these pixels is drawn bigger than them, both ways.
fn magnified(viewport: Viewport, (width, height): (u32, u32)) -> bool {
    width > 0 && height > 0 && viewport.width > width as f32 && viewport.height > height as f32
}

/// the first and last window pixels, left, top, right and bottom, whose
/// centres a viewport covers, kept within the window, none when it covers
/// none.
fn covered_pixels(viewport: Viewport, extent: vk::Extent2D) -> Option<[i32; 4]> {
    let first = |start: f32| (start - 0.5).ceil().max(0.0) as i32;
    let last = |end: f32, size: u32| ((end - 0.5).ceil() as i32 - 1).min(size as i32 - 1);
    let rect = [
        first(viewport.x),
        first(viewport.y),
        last(viewport.x + viewport.width, extent.width),
        last(viewport.y + viewport.height, extent.height),
    ];
    (rect[0] <= rect[2] && rect[1] <= rect[3]).then_some(rect)
}

/// FsrRcasCon's sharpness, from the stops it falls below the sharpest.
fn rcas_sharpness(stops: f32) -> f32 {
    (-stops).exp2()
}

/// push constants of these floats.
fn float_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

impl VulkanPresenter {
    /// the frame's places, each marked with how it is drawn, see through,
    /// with what the FSR filter draws them with made, and Anime4K's passes
    /// let go of once another filter is chosen. should that fail, every
    /// place is drawn as it is, and the filter is EASU alone from then on.
    pub(super) fn fsr_places(&mut self, places: Vec<(Viewport, Shown)>, sizes: &[(u32, u32); 3]) -> Vec<Place> {
        let mut places = self.mark_sharpened(places, sizes);
        let released = match (&mut self.fsr, self.filter.anime4k()) {
            (Some(fsr), None) => fsr.anime4k.release(&self.device),
            _ => Ok(()),
        };
        let filtered = places.iter().any(|place| place.through != Through::Plain);
        let prepared = released.and_then(|()| if filtered { self.prepare_fsr(&places, sizes) } else { Ok(()) });
        if let Err(error) = prepared {
            // the fence was reset, so the frame is drawn anyway, with EASU
            // alone from now on
            log::warn!("the FSR filter will upscale without SMAA or RCAS, {error}");
            if let Some(fsr) = &mut self.fsr {
                fsr.broken = true;
            }
            for place in &mut places {
                place.through = Through::Plain;
            }
        }
        places
    }

    /// the frame's places, each marked with how it is drawn, see through.
    fn mark_sharpened(&self, places: Vec<(Viewport, Shown)>, sizes: &[(u32, u32); 3]) -> Vec<Place> {
        let usable = self.filter.upscales() && self.fsr.as_ref().is_some_and(|fsr| !fsr.broken);
        places
            .into_iter()
            .map(|(viewport, shown)| Place { viewport, shown, through: through(usable, shown, viewport, sizes[screen_index(shown)]) })
            .collect()
    }

    /// makes what the antialiased places need: SMAA's images for each of
    /// their screens at its size, Anime4K's for the sharpened ones when it
    /// is the filter, and the image the sharpened ones are upscaled into.
    fn prepare_fsr(&mut self, places: &[Place], sizes: &[(u32, u32); 3]) -> Result<(), PresentError> {
        let Some(fsr) = &mut self.fsr else { return Ok(()) };
        let mut wanted = [false; 3];
        for place in places {
            wanted[screen_index(place.shown)] |= place.through != Through::Plain;
        }
        for index in 0..3 {
            let size = sizes[index];
            if !wanted[index] || fsr.screens[index].as_ref().is_some_and(|smaa| smaa.size == size) {
                continue;
            }
            // the other frame in flight may still read the old ones
            unsafe { self.device.device_wait_idle() }.map_err(vk_fail("waiting for idle"))?;
            if let Some(old) = fsr.screens[index].take() {
                old.destroy(&self.device, self.descriptor_pool);
            }
            fsr.anime4k.drop_screen(&self.device, index);
            let smaa = fsr.smaa(&self.device, &self.memory_properties, self.descriptor_pool, self.descriptor_layout, self.sampler, size)?;
            fsr.screens[index] = Some(smaa);
        }
        let sharpened = sharpened_screens(places);
        // Anime4K's images for each screen a sharpened place shows, made
        // after SMAA's, whose result they read
        if let Some(chain) = self.filter.anime4k().filter(|_| sharpened.contains(&true)) {
            let mains = std::array::from_fn(|index| {
                fsr.screens[index].as_ref().filter(|_| sharpened[index]).map(|smaa| (smaa.result.texture.view, smaa.size))
            });
            fsr.anime4k.prepare(&self.device, &self.memory_properties, fsr.colour_pass, self.descriptor_layout, self.sampler, chain, mains)?;
        }
        if sharpened.contains(&true) && fsr.upscaled.is_none() {
            let target = Target::new(&self.device, &self.memory_properties, fsr.colour_pass, COLOUR_FORMAT, (self.extent.width, self.extent.height))?;
            let set = picture_set(&self.device, self.descriptor_pool, self.descriptor_layout, self.sampler, &[target.texture.view]);
            match set {
                Ok(set) => fsr.upscaled = Some((target, set)),
                Err(error) => {
                    target.destroy(&self.device);
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

/// an image of the presenter's own on the device, with a view of it.
pub(super) struct Texture {
    pub(super) image: vk::Image,
    memory: vk::DeviceMemory,
    pub(super) view: vk::ImageView,
}

impl Texture {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        (width, height): (u32, u32),
        usage: vk::ImageUsageFlags,
    ) -> Result<Texture, PresentError> {
        let mut texture = Texture { image: vk::Image::null(), memory: vk::DeviceMemory::null(), view: vk::ImageView::null() };
        // what was made goes again, should anything fail
        let made = (|| {
            texture.image = unsafe {
                device.create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(vk::Extent3D { width, height, depth: 1 })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
            }
            .map_err(vk_fail("creating an image"))?;
            let requirements = unsafe { device.get_image_memory_requirements(texture.image) };
            let type_index = find_memory_type(memory_properties, requirements.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or_else(|| fail("no device-local memory for an image"))?;
            texture.memory = unsafe {
                device.allocate_memory(
                    &vk::MemoryAllocateInfo::default().allocation_size(requirements.size).memory_type_index(type_index),
                    None,
                )
            }
            .map_err(vk_fail("allocating image memory"))?;
            unsafe { device.bind_image_memory(texture.image, texture.memory, 0) }.map_err(vk_fail("binding image memory"))?;
            texture.view = unsafe {
                device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(texture.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )
            }
            .map_err(vk_fail("creating an image view"))?;
            Ok(())
        })();
        match made {
            Ok(()) => Ok(texture),
            Err(error) => {
                texture.destroy(device);
                Err(error)
            }
        }
    }

    /// lets go of the image, its memory and its view; those not made are
    /// null, which Vulkan lets go of as nothing.
    pub(super) fn destroy(&self, device: &ash::Device) {
        unsafe {
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

/// an image one of the FSR filter's passes draws into and a later one
/// reads.
pub(super) struct Target {
    pub(super) texture: Texture,
    pub(super) framebuffer: vk::Framebuffer,
}

impl Target {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        render_pass: vk::RenderPass,
        format: vk::Format,
        size: (u32, u32),
    ) -> Result<Target, PresentError> {
        // the tests read what was drawn back
        let read_back = if cfg!(test) { vk::ImageUsageFlags::TRANSFER_SRC } else { vk::ImageUsageFlags::empty() };
        let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED | read_back;
        let texture = Texture::new(device, memory_properties, format, size, usage)?;
        let attachments = [texture.view];
        let framebuffer = unsafe {
            device.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(render_pass)
                    .attachments(&attachments)
                    .width(size.0)
                    .height(size.1)
                    .layers(1),
                None,
            )
        };
        match framebuffer {
            Ok(framebuffer) => Ok(Target { texture, framebuffer }),
            Err(error) => {
                texture.destroy(device);
                Err(vk_fail("creating a framebuffer")(error))
            }
        }
    }

    pub(super) fn destroy(&self, device: &ash::Device) {
        unsafe { device.destroy_framebuffer(self.framebuffer, None) };
        self.texture.destroy(device);
    }
}

/// a set of the layout's with these pictures at bindings 0 on, read in the
/// shader read only layout through the sampler.
pub(super) fn picture_set(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    views: &[vk::ImageView],
) -> Result<vk::DescriptorSet, PresentError> {
    let layouts = [layout];
    let set = unsafe {
        device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts))
    }
    .map_err(vk_fail("allocating a descriptor set"))?[0];
    let infos: Vec<[vk::DescriptorImageInfo; 1]> = views
        .iter()
        .map(|&view| {
            [vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)]
        })
        .collect();
    let writes: Vec<vk::WriteDescriptorSet> = infos
        .iter()
        .enumerate()
        .map(|(binding, info)| {
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(binding as u32)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(info)
        })
        .collect();
    // SAFETY: the set was just made, no command uses it
    unsafe { device.update_descriptor_sets(&writes, &[]) };
    Ok(set)
}

/// SMAA's images for one screen at its size, each with the set the next
/// pass reads it through.
pub(super) struct Smaa {
    size: (u32, u32),
    edges: Target,
    weights: Target,
    result: Target,
    /// the edges and the area and search tables, for the weights.
    weights_inputs: vk::DescriptorSet,
    /// the weights, for the blending.
    blend_input: vk::DescriptorSet,
    /// the antialiased screen, for EASU.
    pub(super) result_input: vk::DescriptorSet,
}

impl Smaa {
    fn destroy(self, device: &ash::Device, pool: vk::DescriptorPool) {
        unsafe {
            let _ = device.free_descriptor_sets(pool, &[self.weights_inputs, self.blend_input, self.result_input]);
        }
        self.edges.destroy(device);
        self.weights.destroy(device);
        self.result.destroy(device);
    }
}

/// what the FSR filter draws with besides the present shader. a screen it
/// draws bigger than its pixels is antialiased with SMAA 1x at its own size
/// (edges, blending weights, neighbourhood blending), upscaled with the
/// present shader's EASU into an image the window's size, and sharpened
/// from that onto the window with RCAS, under the overlay.
///
/// one set of images serves both frames in flight: every pass is on the one
/// queue, and each render pass waits, through its dependencies, for what
/// was submitted before it to be done reading or writing its image.
pub(super) struct Fsr {
    edges_pass: vk::RenderPass,
    /// the render pass of every colour image, Anime4K's included.
    colour_pass: vk::RenderPass,
    /// the edges and the two tables, for the weights.
    lookup_layout: vk::DescriptorSetLayout,
    edges: Pipeline,
    weights: Pipeline,
    blend: Pipeline,
    /// the present shader, drawing into the upscaled image.
    easu: Pipeline,
    rcas: Pipeline,
    area: Texture,
    search: Texture,
    /// SMAA's images of the top screen, the bottom one and the right eye's,
    /// made when first wanted and again when the screen's size changes.
    pub(super) screens: [Option<Smaa>; 3],
    /// the screens upscaled, the window's size, and the set reading them,
    /// made when first wanted and again with the swapchain.
    upscaled: Option<(Target, vk::DescriptorSet)>,
    /// set when making the images failed, after which the filter is EASU
    /// alone.
    broken: bool,
    /// Anime4K's passes, between SMAA and EASU while it is the filter.
    anime4k: Anime4kState,
}

impl Fsr {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        command_pool: vk::CommandPool,
        window_pass: vk::RenderPass,
        picture_layout: vk::DescriptorSetLayout,
    ) -> Result<Fsr, PresentError> {
        let none = || Texture { image: vk::Image::null(), memory: vk::DeviceMemory::null(), view: vk::ImageView::null() };
        let mut fsr = Fsr {
            edges_pass: vk::RenderPass::null(),
            colour_pass: vk::RenderPass::null(),
            lookup_layout: vk::DescriptorSetLayout::null(),
            edges: NO_PIPELINE,
            weights: NO_PIPELINE,
            blend: NO_PIPELINE,
            easu: NO_PIPELINE,
            rcas: NO_PIPELINE,
            area: none(),
            search: none(),
            screens: [None, None, None],
            upscaled: None,
            broken: false,
            anime4k: Anime4kState::Unmade,
        };
        // what was made goes again, should anything fail
        let made = (|| {
            fsr.edges_pass = create_target_pass(device, EDGES_FORMAT)?;
            fsr.colour_pass = create_target_pass(device, COLOUR_FORMAT)?;
            fsr.lookup_layout = create_descriptor_layout(device, 3)?;
            fsr.edges = create_pipeline(device, fsr.edges_pass, &[picture_layout], SMAA_EDGES_SPIRV, SMAA_CONSTANTS)?;
            fsr.weights = create_pipeline(device, fsr.colour_pass, &[fsr.lookup_layout], SMAA_WEIGHTS_SPIRV, SMAA_CONSTANTS)?;
            // the screen in set 0, as the first pass reads it, the weights in 1
            fsr.blend = create_pipeline(device, fsr.colour_pass, &[picture_layout, picture_layout], SMAA_BLEND_SPIRV, SMAA_CONSTANTS)?;
            fsr.easu = create_pipeline(device, fsr.colour_pass, &[picture_layout], FRAGMENT_SPIRV, CONSTANTS)?;
            fsr.rcas = create_pipeline(device, window_pass, &[picture_layout], RCAS_SPIRV, RCAS_CONSTANTS)?;
            let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
            fsr.area = Texture::new(device, memory_properties, vk::Format::R8G8_UNORM, SMAA_AREA_SIZE, usage)?;
            fsr.search = Texture::new(device, memory_properties, vk::Format::R8_UNORM, SMAA_SEARCH_SIZE, usage)?;
            upload_tables(device, memory_properties, queue, command_pool, &fsr.area, &fsr.search)
        })();
        match made {
            Ok(()) => Ok(fsr),
            Err(error) => {
                fsr.destroy_passes(device);
                Err(error)
            }
        }
    }

    /// SMAA's images for a screen of this size.
    fn smaa(
        &self,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        pool: vk::DescriptorPool,
        picture_layout: vk::DescriptorSetLayout,
        sampler: vk::Sampler,
        size: (u32, u32),
    ) -> Result<Smaa, PresentError> {
        let (mut targets, mut sets) = (Vec::new(), Vec::new());
        // what was made goes again, should anything fail
        let made = (|| {
            for (render_pass, format) in [(self.edges_pass, EDGES_FORMAT), (self.colour_pass, COLOUR_FORMAT), (self.colour_pass, COLOUR_FORMAT)] {
                targets.push(Target::new(device, memory_properties, render_pass, format, size)?);
            }
            let tables = [targets[0].texture.view, self.area.view, self.search.view];
            sets.push(picture_set(device, pool, self.lookup_layout, sampler, &tables)?);
            sets.push(picture_set(device, pool, picture_layout, sampler, &[targets[1].texture.view])?);
            sets.push(picture_set(device, pool, picture_layout, sampler, &[targets[2].texture.view])?);
            Ok(())
        })();
        if let Err(error) = made {
            if !sets.is_empty() {
                unsafe {
                    let _ = device.free_descriptor_sets(pool, &sets);
                }
            }
            targets.iter().for_each(|target| target.destroy(device));
            return Err(error);
        }
        let mut targets = targets.into_iter();
        let (Some(edges), Some(weights), Some(result)) = (targets.next(), targets.next(), targets.next()) else { unreachable!("three targets made") };
        Ok(Smaa { size, edges, weights, result, weights_inputs: sets[0], blend_input: sets[1], result_input: sets[2] })
    }

    pub(super) fn drop_upscaled(&mut self, device: &ash::Device, pool: vk::DescriptorPool) {
        if let Some((target, set)) = self.upscaled.take() {
            unsafe {
                let _ = device.free_descriptor_sets(pool, &[set]);
            }
            target.destroy(device);
        }
    }

    /// records SMAA's passes over each antialiased place's screen, once
    /// however many places show it, Anime4K's after them for a sharpened
    /// one while it is the filter, then EASU's of every sharpened place into
    /// the upscaled image, in the window's places.
    pub(super) fn record(
        &self,
        device: &ash::Device,
        command_buffer: vk::CommandBuffer,
        places: &[Place],
        crops: &[Option<(vk::DescriptorSet, [f32; 8])>; 3],
        extent: vk::Extent2D,
    ) {
        let sharpened = sharpened_screens(places);
        // Anime4K's picture of each screen a sharpened place shows, twice
        // as big
        let doubled = |index: usize| self.anime4k.made().filter(|_| sharpened[index]).and_then(|anime4k| anime4k.doubled(index));
        let mut done = [false; 3];
        for place in places {
            let index = screen_index(place.shown);
            if place.through == Through::Plain || std::mem::replace(&mut done[index], true) {
                continue;
            }
            let (Some(smaa), Some((source, area))) = (&self.screens[index], crops[index]) else { continue };
            // where the screen lies in its image, then SMAA's metrics of it
            let (width, height) = smaa.size;
            let mut values = area.to_vec();
            values.extend([1.0 / width as f32, 1.0 / height as f32, width as f32, height as f32]);
            let constants = float_bytes(&values);
            let size = vk::Extent2D { width, height };
            let whole = Viewport { x: 0.0, y: 0.0, width: width as f32, height: height as f32 };
            let (edges_sets, weights_sets, blend_sets) = ([source], [smaa.weights_inputs], [source, smaa.blend_input]);
            let passes: [(vk::RenderPass, &Target, Pipeline, &[vk::DescriptorSet]); 3] = [
                (self.edges_pass, &smaa.edges, self.edges, &edges_sets),
                (self.colour_pass, &smaa.weights, self.weights, &weights_sets),
                (self.colour_pass, &smaa.result, self.blend, &blend_sets),
            ];
            for (render_pass, target, (layout, pipeline), sets) in passes {
                begin_pass(device, command_buffer, render_pass, target.framebuffer, size);
                set_viewport(device, command_buffer, whole, size);
                unsafe {
                    device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
                    device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::GRAPHICS, layout, 0, sets, &[]);
                    device.cmd_push_constants(command_buffer, layout, vk::ShaderStageFlags::FRAGMENT, 0, &constants);
                    device.cmd_draw(command_buffer, 3, 1, 0, 0);
                    device.cmd_end_render_pass(command_buffer);
                }
            }
            if let Some(anime4k) = self.anime4k.made().filter(|_| sharpened[index]) {
                anime4k.record(device, command_buffer, index);
            }
        }

        // the whole of each screen drawn bigger, through EASU
        let Some((upscaled, _)) = self.upscaled.as_ref().filter(|_| sharpened.contains(&true)) else { return };
        let mut constants = float_bytes(&WHOLE);
        constants.extend_from_slice(&ScreenFilter::Fsr.mode().to_le_bytes());
        let (layout, pipeline) = self.easu;
        begin_pass(device, command_buffer, self.colour_pass, upscaled.framebuffer, extent);
        for place in places {
            let index = screen_index(place.shown);
            let Some(smaa) = self.screens[index].as_ref().filter(|_| place.through == Through::Sharpened) else { continue };
            // Anime4K's picture twice as big, or SMAA's
            let input = doubled(index).unwrap_or(smaa.result_input);
            set_viewport(device, command_buffer, place.viewport, extent);
            unsafe {
                device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
                device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[input], &[]);
                device.cmd_push_constants(command_buffer, layout, vk::ShaderStageFlags::FRAGMENT, 0, &constants);
                device.cmd_draw(command_buffer, 3, 1, 0, 0);
            }
        }
        unsafe { device.cmd_end_render_pass(command_buffer) };
    }

    /// draws a sharpened place onto the window from the upscaled image,
    /// within the window's render pass.
    pub(super) fn sharpen(&self, device: &ash::Device, command_buffer: vk::CommandBuffer, viewport: Viewport, extent: vk::Extent2D) {
        let (Some((_, set)), Some(rect)) = (&self.upscaled, covered_pixels(viewport, extent)) else { return };
        let mut constants: Vec<u8> = rect.iter().flat_map(|value| value.to_le_bytes()).collect();
        constants.extend_from_slice(&rcas_sharpness(RCAS_SHARPNESS_STOPS).to_le_bytes());
        let (layout, pipeline) = self.rcas;
        set_viewport(device, command_buffer, viewport, extent);
        unsafe {
            device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
            device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[*set], &[]);
            device.cmd_push_constants(command_buffer, layout, vk::ShaderStageFlags::FRAGMENT, 0, &constants);
            device.cmd_draw(command_buffer, 3, 1, 0, 0);
        }
    }

    pub(super) fn destroy(mut self, device: &ash::Device, pool: vk::DescriptorPool) {
        for smaa in self.screens.iter_mut().filter_map(Option::take) {
            smaa.destroy(device, pool);
        }
        self.drop_upscaled(device, pool);
        self.anime4k.destroy(device);
        self.destroy_passes(device);
    }

    /// lets go of what Fsr::new made; what it did not make is null, which
    /// Vulkan lets go of as nothing.
    fn destroy_passes(&self, device: &ash::Device) {
        self.area.destroy(device);
        self.search.destroy(device);
        unsafe {
            for (layout, pipeline) in [self.edges, self.weights, self.blend, self.easu, self.rcas] {
                device.destroy_pipeline(pipeline, None);
                device.destroy_pipeline_layout(layout, None);
            }
            device.destroy_descriptor_set_layout(self.lookup_layout, None);
            device.destroy_render_pass(self.edges_pass, None);
            device.destroy_render_pass(self.colour_pass, None);
        }
    }
}

/// begins a render pass over the whole of a framebuffer, cleared to black,
/// which for SMAA's edges is no edge.
pub(super) fn begin_pass(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    extent: vk::Extent2D,
) {
    let clear = [vk::ClearValue { color: vk::ClearColorValue { float32: [0.0, 0.0, 0.0, 1.0] } }];
    unsafe {
        device.cmd_begin_render_pass(
            command_buffer,
            &vk::RenderPassBeginInfo::default()
                .render_pass(render_pass)
                .framebuffer(framebuffer)
                .render_area(vk::Rect2D { offset: vk::Offset2D { x: 0, y: 0 }, extent })
                .clear_values(&clear),
            vk::SubpassContents::INLINE,
        );
    }
}

/// a render pass drawing into one of the FSR filter's images, cleared,
/// which the passes after read in their fragment shaders.
pub(super) fn create_target_pass(device: &ash::Device, format: vk::Format) -> Result<vk::RenderPass, PresentError> {
    let attachments = [vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
    let color = [vk::AttachmentReference::default().attachment(0).layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color)];
    let dependencies = [
        // after whatever read or wrote the image before, in this frame or
        // the one before it
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
        // before the next pass's fragment shaders read it
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags::SHADER_READ),
    ];
    unsafe {
        device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&dependencies),
            None,
        )
    }
    .map_err(vk_fail("creating a render pass"))
}

/// copies SMAA's area and search tables up into their images, once, and
/// waits for them.
fn upload_tables(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    area: &Texture,
    search: &Texture,
) -> Result<(), PresentError> {
    // both in one buffer, the search table after the area one
    let size = (SMAA_AREA.len() + SMAA_SEARCH.len()) as vk::DeviceSize;
    let (mut staging, mut memory) = (vk::Buffer::null(), vk::DeviceMemory::null());
    let uploaded = (|| {
        staging = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(vk_fail("creating a staging buffer"))?;
        let requirements = unsafe { device.get_buffer_memory_requirements(staging) };
        let type_index = find_memory_type(
            memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| fail("no host-visible memory for a staging buffer"))?;
        memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default().allocation_size(requirements.size).memory_type_index(type_index),
                None,
            )
        }
        .map_err(vk_fail("allocating staging memory"))?;
        unsafe { device.bind_buffer_memory(staging, memory, 0) }.map_err(vk_fail("binding staging memory"))?;
        let mapping = unsafe { device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty()) }
            .map_err(vk_fail("mapping staging memory"))? as *mut u8;
        // SAFETY: the mapping is size bytes long, host coherent, and nothing
        // else uses it
        unsafe {
            std::ptr::copy_nonoverlapping(SMAA_AREA.as_ptr(), mapping, SMAA_AREA.len());
            std::ptr::copy_nonoverlapping(SMAA_SEARCH.as_ptr(), mapping.add(SMAA_AREA.len()), SMAA_SEARCH.len());
            device.unmap_memory(memory);
        }

        let command_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(vk_fail("allocating a command buffer"))?[0];
        let buffers = [command_buffer];
        let copies = [(area, SMAA_AREA_SIZE, 0), (search, SMAA_SEARCH_SIZE, SMAA_AREA.len())];
        let submitted = unsafe {
            device
                .begin_command_buffer(command_buffer, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))
                .map_err(vk_fail("beginning a command buffer"))
                .and_then(|()| {
                    for (texture, (width, height), offset) in copies {
                        transition(device, command_buffer, texture.image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                        device.cmd_copy_buffer_to_image(
                            command_buffer,
                            staging,
                            texture.image,
                            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            &[vk::BufferImageCopy::default()
                                .buffer_offset(offset as vk::DeviceSize)
                                .image_subresource(
                                    vk::ImageSubresourceLayers::default()
                                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                                        .layer_count(1),
                                )
                                .image_extent(vk::Extent3D { width, height, depth: 1 })],
                        );
                        transition(
                            device,
                            command_buffer,
                            texture.image,
                            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        );
                    }
                    device.end_command_buffer(command_buffer).map_err(vk_fail("ending a command buffer"))
                })
                .and_then(|()| {
                    device
                        .queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&buffers)], vk::Fence::null())
                        .map_err(vk_fail("submitting the SMAA tables"))
                })
                .and_then(|()| device.queue_wait_idle(queue).map_err(vk_fail("waiting for the SMAA tables")))
        };
        unsafe { device.free_command_buffers(command_pool, &buffers) };
        submitted
    })();
    // null when not made, which Vulkan lets go of as nothing
    unsafe {
        device.destroy_buffer(staging, None);
        device.free_memory(memory, None);
    }
    uploaded
}

/// the GPU the tests draw on, when there is one, with what they draw with.
#[cfg(test)]
pub(super) mod testing {
    use super::*;

    /// the device, a command pool, a one-picture set layout and a linear
    /// sampler, let go of when dropped.
    pub(in crate::backend::vulkan) struct TestGpu {
        pub(in crate::backend::vulkan) shared: crate::SharedDevice,
        pub(in crate::backend::vulkan) memory_properties: vk::PhysicalDeviceMemoryProperties,
        pub(in crate::backend::vulkan) pool: vk::CommandPool,
        pub(in crate::backend::vulkan) picture_layout: vk::DescriptorSetLayout,
        pub(in crate::backend::vulkan) sampler: vk::Sampler,
    }

    impl TestGpu {
        /// the GPU, or none, said as a test skipped, when there is none.
        pub(in crate::backend::vulkan) fn new() -> Option<TestGpu> {
            let shared = match crate::SharedDevice::new() {
                Ok(shared) => shared,
                Err(error) => {
                    eprintln!("skipped, {error}");
                    return None;
                }
            };
            let device = &shared.device;
            let memory_properties = unsafe { shared.instance.get_physical_device_memory_properties(shared.physical) };
            let pool = unsafe { device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(shared.family), None) }.unwrap();
            let picture_layout = create_descriptor_layout(device, 1).unwrap();
            let sampler_info = vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
            let sampler = unsafe { device.create_sampler(&sampler_info, None) }.unwrap();
            Some(TestGpu { shared, memory_properties, pool, picture_layout, sampler })
        }

        pub(in crate::backend::vulkan) fn device(&self) -> &ash::Device {
            &self.shared.device
        }

        /// an image of RGBA8 pixels of this size, read in the shader read
        /// only layout.
        pub(in crate::backend::vulkan) fn upload(&self, pixels: &[u8], size: (u32, u32)) -> Texture {
            let device = self.device();
            let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
            let texture = Texture::new(device, &self.memory_properties, COLOUR_FORMAT, size, usage).unwrap();
            let staging = self.host_buffer(pixels.len(), vk::BufferUsageFlags::TRANSFER_SRC);
            unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), staging.2, pixels.len()) };
            self.run(|command_buffer| unsafe {
                transition(device, command_buffer, texture.image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                let layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
                device.cmd_copy_buffer_to_image(command_buffer, staging.0, texture.image, layout, &[whole(size)]);
                transition(device, command_buffer, texture.image, layout, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            });
            self.free_buffer(staging);
            texture
        }

        /// records commands, runs them and waits for them.
        pub(in crate::backend::vulkan) fn run(&self, record: impl FnOnce(vk::CommandBuffer)) {
            let device = self.device();
            unsafe {
                let info = vk::CommandBufferAllocateInfo::default().command_pool(self.pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
                let buffers = device.allocate_command_buffers(&info).unwrap();
                let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
                device.begin_command_buffer(buffers[0], &begin).unwrap();
                record(buffers[0]);
                device.end_command_buffer(buffers[0]).unwrap();
                device.queue_submit(self.shared.queue, &[vk::SubmitInfo::default().command_buffers(&buffers)], vk::Fence::null()).unwrap();
                device.queue_wait_idle(self.shared.queue).unwrap();
                device.free_command_buffers(self.pool, &buffers);
            }
        }

        /// the RGBA8 pixels of an image a pass of the filter's drew.
        pub(in crate::backend::vulkan) fn read_back(&self, image: vk::Image, size: (u32, u32)) -> Vec<u8> {
            let device = self.device();
            let bytes = (size.0 * size.1 * 4) as usize;
            let buffer = self.host_buffer(bytes, vk::BufferUsageFlags::TRANSFER_DST);
            self.run(|command_buffer| unsafe {
                let barrier = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(1))
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                let stages = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT;
                device.cmd_pipeline_barrier(command_buffer, stages, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[barrier]);
                device.cmd_copy_image_to_buffer(command_buffer, image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, buffer.0, &[whole(size)]);
                let host = vk::MemoryBarrier::default().src_access_mask(vk::AccessFlags::TRANSFER_WRITE).dst_access_mask(vk::AccessFlags::HOST_READ);
                device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::HOST, vk::DependencyFlags::empty(), &[host], &[], &[]);
            });
            let pixels = unsafe { std::slice::from_raw_parts(buffer.2, bytes) }.to_vec();
            self.free_buffer(buffer);
            pixels
        }

        /// a host visible buffer, mapped.
        fn host_buffer(&self, size: usize, usage: vk::BufferUsageFlags) -> (vk::Buffer, vk::DeviceMemory, *mut u8) {
            let device = self.device();
            unsafe {
                let info = vk::BufferCreateInfo::default().size(size as u64).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE);
                let buffer = device.create_buffer(&info, None).unwrap();
                let requirements = device.get_buffer_memory_requirements(buffer);
                let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
                let index = find_memory_type(&self.memory_properties, requirements.memory_type_bits, wanted).unwrap();
                let allocation = vk::MemoryAllocateInfo::default().allocation_size(requirements.size).memory_type_index(index);
                let memory = device.allocate_memory(&allocation, None).unwrap();
                device.bind_buffer_memory(buffer, memory, 0).unwrap();
                let mapping = device.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty()).unwrap() as *mut u8;
                (buffer, memory, mapping)
            }
        }

        fn free_buffer(&self, (buffer, memory, _): (vk::Buffer, vk::DeviceMemory, *mut u8)) {
            let device = self.device();
            unsafe {
                device.unmap_memory(memory);
                device.destroy_buffer(buffer, None);
                device.free_memory(memory, None);
            }
        }
    }

    impl Drop for TestGpu {
        fn drop(&mut self) {
            let device = &self.shared.device;
            unsafe {
                device.destroy_sampler(self.sampler, None);
                device.destroy_descriptor_set_layout(self.picture_layout, None);
                device.destroy_command_pool(self.pool, None);
            }
        }
    }

    /// a whole RGBA8 image of this size, copied to or from a buffer.
    fn whole((width, height): (u32, u32)) -> vk::BufferImageCopy {
        vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1))
            .image_extent(vk::Extent3D { width, height, depth: 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TestGpu;
    use super::*;

    #[test]
    fn the_smaa_tables_are_the_size_the_shaders_read() {
        assert_eq!(SMAA_AREA.len() as u32, SMAA_AREA_SIZE.0 * SMAA_AREA_SIZE.1 * 2);
        assert_eq!(SMAA_SEARCH.len() as u32, SMAA_SEARCH_SIZE.0 * SMAA_SEARCH_SIZE.1);
        // the search table's copy starts where a copy may
        assert_eq!(SMAA_AREA.len() % 4, 0);
    }

    #[test]
    fn rcas_sharpness_halves_with_each_stop() {
        assert_eq!(rcas_sharpness(0.0), 1.0);
        assert_eq!(rcas_sharpness(1.0), 0.5);
        assert_eq!(rcas_sharpness(2.0), 0.25);
        assert!((rcas_sharpness(RCAS_SHARPNESS_STOPS) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn only_a_screen_drawn_bigger_both_ways_is_magnified() {
        let viewport = |width, height| Viewport { x: 0.0, y: 0.0, width, height };
        assert!(magnified(viewport(1120.0, 672.0), (800, 480)));
        assert!(!magnified(viewport(800.0, 480.0), (800, 480)), "1:1 draws as before");
        assert!(!magnified(viewport(600.0, 360.0), (800, 480)), "shrunk draws as before");
        assert!(!magnified(viewport(1120.0, 470.0), (800, 480)), "shrunk one way draws as before");
        assert!(!magnified(viewport(1120.0, 672.0), (0, 0)), "nothing to draw");
    }

    /// the FSR filter antialiases a screen however big it is drawn, and
    /// upscales and sharpens it only when it is drawn bigger; the top
    /// screen's two pictures in one for 3D, and the other filters, draw as
    /// they are.
    #[test]
    fn the_fsr_filter_antialiases_a_screen_at_any_size() {
        let viewport = |width, height| Viewport { x: 0.0, y: 0.0, width, height };
        assert_eq!(through(true, Shown::Top, viewport(1200.0, 720.0), (800, 480)), Through::Sharpened);
        assert_eq!(through(true, Shown::Top, viewport(1200.0, 720.0), (1200, 720)), Through::Smaa, "1:1");
        assert_eq!(through(true, Shown::Bottom, viewport(800.0, 480.0), (1200, 720)), Through::Smaa, "shrunk");
        assert_eq!(through(true, Shown::Both, viewport(1200.0, 720.0), (800, 480)), Through::Plain, "3D");
        assert_eq!(through(false, Shown::Top, viewport(1200.0, 720.0), (800, 480)), Through::Plain, "another filter");
        assert_eq!(through(true, Shown::Top, viewport(1200.0, 720.0), (0, 0)), Through::Plain, "nothing to draw");
    }

    #[test]
    fn the_pixels_covered_are_those_whose_centres_the_viewport_holds() {
        let extent = vk::Extent2D { width: 1280, height: 720 };
        let viewport = |x, y, width, height| Viewport { x, y, width, height };
        assert_eq!(covered_pixels(viewport(0.0, 0.0, 1280.0, 720.0), extent), Some([0, 0, 1279, 719]));
        assert_eq!(covered_pixels(viewport(240.5, 10.25, 800.0, 480.0), extent), Some([240, 10, 1039, 489]));
        assert_eq!(covered_pixels(viewport(-100.0, 600.0, 400.0, 240.0), extent), Some([0, 600, 299, 719]), "kept within the window");
        assert_eq!(covered_pixels(viewport(10.0, 10.0, 0.25, 0.25), extent), None);
    }

    #[test]
    fn each_place_reads_its_screen() {
        assert_eq!([Shown::Top, Shown::Both, Shown::Bottom, Shown::Right].map(screen_index), [0, 0, 1, 2]);
    }

    /// a screen is sharpened when any place showing it is, the top one's
    /// for its two pictures in one too.
    #[test]
    fn the_sharpened_screens_are_those_a_sharpened_place_shows() {
        let viewport = Viewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };
        let place = |shown, through| Place { viewport, shown, through };
        assert_eq!(sharpened_screens(&[]), [false; 3]);
        let places = [place(Shown::Top, Through::Smaa), place(Shown::Bottom, Through::Sharpened), place(Shown::Right, Through::Plain)];
        assert_eq!(sharpened_screens(&places), [false, true, false]);
        assert_eq!(sharpened_screens(&[place(Shown::Both, Through::Sharpened)]), [true, false, false]);
    }

    /// on the GPU, when there is one: a screen of a hard, shallow line
    /// comes out of SMAA blended along the line alone, and out of EASU and
    /// RCAS twice its size with its flat parts kept.
    #[test]
    fn the_fsr_filter_antialiases_upscales_and_sharpens_a_screen() {
        let Some(gpu) = TestGpu::new() else { return };
        let (device, memory_properties) = (gpu.device(), &gpu.memory_properties);
        let sizes = [vk::DescriptorPoolSize::default().ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(8)];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .pool_sizes(&sizes)
            .max_sets(6);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }.unwrap();
        // the window's pass, compatible with the swapchain's but read back
        let window_pass = create_target_pass(device, COLOUR_FORMAT).unwrap();
        let mut fsr = Fsr::new(device, memory_properties, gpu.shared.queue, gpu.pool, window_pass, gpu.picture_layout).unwrap();

        // white over black, the line one row lower every 8 pixels
        let (width, height) = (64u32, 64u32);
        let edge = |x: u32| 20 + x / 8;
        let pixels: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| if y < edge(x) { [255; 4] } else { [0, 0, 0, 255] }))
            .collect();
        let screen = gpu.upload(&pixels, (width, height));
        let screen_set = picture_set(device, descriptor_pool, gpu.picture_layout, gpu.sampler, &[screen.view]).unwrap();

        // drawn at twice its size
        let smaa = fsr.smaa(device, memory_properties, descriptor_pool, gpu.picture_layout, gpu.sampler, (width, height)).unwrap();
        fsr.screens[0] = Some(smaa);
        let extent = vk::Extent2D { width: width * 2, height: height * 2 };
        let upscaled = Target::new(device, memory_properties, fsr.colour_pass, COLOUR_FORMAT, (extent.width, extent.height)).unwrap();
        let upscaled_set = picture_set(device, descriptor_pool, gpu.picture_layout, gpu.sampler, &[upscaled.texture.view]).unwrap();
        fsr.upscaled = Some((upscaled, upscaled_set));
        let window = Target::new(device, memory_properties, window_pass, COLOUR_FORMAT, (extent.width, extent.height)).unwrap();
        let viewport = Viewport { x: 0.0, y: 0.0, width: extent.width as f32, height: extent.height as f32 };
        assert!(magnified(viewport, (width, height)));
        let places = [Place { viewport, shown: Shown::Top, through: Through::Sharpened }];
        let crops = [Some((screen_set, WHOLE)), None, None];
        gpu.run(|command_buffer| {
            fsr.record(device, command_buffer, &places, &crops, extent);
            begin_pass(device, command_buffer, window_pass, window.framebuffer, extent);
            fsr.sharpen(device, command_buffer, viewport, extent);
            unsafe { device.cmd_end_render_pass(command_buffer) };
        });

        let result = fsr.screens[0].as_ref().unwrap().result.texture.image;
        let antialiased = gpu.read_back(result, (width, height));
        let shown = gpu.read_back(window.texture.image, (extent.width, extent.height));
        let red = |image: &[u8], across: u32, x: u32, y: u32| image[((y * across + x) * 4) as usize];

        let mut blended = 0;
        for y in 0..height {
            for x in 0..width {
                let value = red(&antialiased, width, x, y);
                if y + 2 < edge(x) || y > edge(x) + 1 {
                    let was = if y < edge(x) { 255 } else { 0 };
                    assert_eq!(value, was, "({x}, {y}) is away from the line");
                } else if value != 0 && value != 255 {
                    blended += 1;
                }
            }
        }
        // each of the 8 steps ramps over several pixels, 88 on one GPU
        assert!(blended >= 48, "only {blended} pixels along the line were blended");

        assert_eq!(red(&shown, extent.width, 4, 4), 255, "white stays white");
        assert_eq!(red(&shown, extent.width, extent.width - 4, extent.height - 4), 0, "black stays black");
        let between = shown.chunks(4).filter(|pixel| pixel[0] != 0 && pixel[0] != 255).count();
        assert!(between > 0, "the line is drawn");

        fsr.destroy(device, descriptor_pool);
        window.destroy(device);
        screen.destroy(device);
        unsafe {
            device.destroy_descriptor_pool(descriptor_pool, None);
            device.destroy_render_pass(window_pass, None);
        }
    }
}
