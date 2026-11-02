//! Vulkan presentation backend.

use std::ffi::CStr;
use std::mem::ManuallyDrop;
use std::sync::Arc;

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use super::vulkan_overlay::OverlayPainter;
use super::{place, GpuScreen, Overlay, PresentError, Presenter, ScreenFilter, ScreenImage, ScreenLayout, Shown, Stereo, StereoMode, Viewport};
use crate::raster::hardware::{can_render, render_device};
use crate::SharedDevice;

const VERTEX_SPIRV: &[u8] = include_bytes!("../../shaders/present.vert.spv");
const FRAGMENT_SPIRV: &[u8] = include_bytes!("../../shaders/present.frag.spv");
const STEREO_FRAGMENT_SPIRV: &[u8] = include_bytes!("../../shaders/present_stereo.frag.spv");
const SMAA_EDGES_SPIRV: &[u8] = include_bytes!("../../shaders/smaa_edges.frag.spv");
const SMAA_WEIGHTS_SPIRV: &[u8] = include_bytes!("../../shaders/smaa_weights.frag.spv");
const SMAA_BLEND_SPIRV: &[u8] = include_bytes!("../../shaders/smaa_blend.frag.spv");
const RCAS_SPIRV: &[u8] = include_bytes!("../../shaders/rcas.frag.spv");

/// SMAA's precomputed tables, from its Textures/AreaTex.h and SearchTex.h:
/// the area each pattern of edges covers, two channels, and how far a
/// search's last step goes, one channel.
const SMAA_AREA: &[u8] = include_bytes!("../../shaders/smaa_area.lut");
const SMAA_AREA_SIZE: (u32, u32) = (160, 560);
const SMAA_SEARCH: &[u8] = include_bytes!("../../shaders/smaa_search.lut");
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
const COLOUR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const EDGES_FORMAT: vk::Format = vk::Format::R8G8_UNORM;

/// the bytes of push constants the fragment shader reads: where the screen
/// lies in its image, the bounds its samples keep in and the filter, and
/// for 3D how the eyes show and the right eye's place in its image.
const CONSTANTS: u32 = 36;
const STEREO_CONSTANTS: u32 = 80;

/// how many frames may be recorded before waiting on the oldest.
const FRAMES_IN_FLIGHT: usize = 2;

/// a whole image, as the area of it a screen takes and the bounds its
/// samples keep within, for the fragment shader.
const WHOLE: [f32; 8] = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0];

/// what a screen is drawn from in a frame.
#[derive(Clone, Copy)]
enum Source {
    Nothing,
    /// the image of its own the presenter uploads pixels to.
    Own,
    /// an image of the renderer's, on the device they share.
    Gpu(GpuScreen),
}

pub(super) fn fail(message: impl Into<String>) -> PresentError {
    PresentError::Backend(message.into())
}

pub(super) fn vk_fail(context: &str) -> impl Fn(vk::Result) -> PresentError + '_ {
    move |error| PresentError::Backend(format!("{context}: {error}"))
}

/// everything belonging to one emulated screen.
struct Screen {
    width: u32,
    height: u32,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    staging: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    staging_mapping: *mut u8,
    staging_size: vk::DeviceSize,
    descriptor: vk::DescriptorSet,
    /// false until the image has been transitioned out of UNDEFINED.
    initialized: bool,
    /// set when new pixels were written into the staging buffer this frame.
    dirty: bool,
    /// the image holds what the screen showed last, not one of the renderer's.
    showing: bool,
}

pub struct VulkanPresenter {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_instance: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,

    physical_device: vk::PhysicalDevice,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    device: ash::Device,
    queue_family: u32,
    queue: vk::Queue,

    swapchain_device: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    surface_format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    image_views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,

    render_pass: vk::RenderPass,
    descriptor_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    /// what draws the top screen's two pictures in one, for 3D in red and
    /// cyan or on alternate rows, with a set per frame in flight.
    stereo_layout: vk::DescriptorSetLayout,
    stereo_pipeline_layout: vk::PipelineLayout,
    stereo_pipeline: vk::Pipeline,
    stereo_sets: Vec<vk::DescriptorSet>,
    sampler: vk::Sampler,
    /// a set per frame in flight and screen, pointed at the renderer's
    /// images when screens are drawn straight from them.
    gpu_sets: Vec<[vk::DescriptorSet; 3]>,
    /// the device and instance, when the renderer draws on them too, which
    /// go with the last of the two rather than with this.
    shared: Option<Arc<SharedDevice>>,
    /// whether a present waits for the display, as FIFO does.
    waits: bool,

    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available: Vec<vk::Semaphore>,
    render_finished: Vec<vk::Semaphore>,
    in_flight: Vec<vk::Fence>,
    frame: usize,

    /// the top screen, the bottom one and the top one's picture for the
    /// right eye.
    screens: [Screen; 3],
    /// what is drawn over the screens, none only while being dropped.
    overlay: Option<OverlayPainter>,
    /// what the FSR filter draws with besides the present shader, none
    /// when it could not be made, which leaves the filter EASU alone.
    fsr: Option<Fsr>,
    window: (u32, u32),
    arrangement: ScreenLayout,
    top_width: f32,
    filter: ScreenFilter,
    integer: bool,
    stereo: Stereo,
    /// set when the swapchain no longer matches the window.
    stale: bool,
}

impl VulkanPresenter {
    pub fn new(
        window: &(impl HasDisplayHandle + HasWindowHandle),
        size: (u32, u32),
    ) -> Result<VulkanPresenter, PresentError> {
        // SAFETY: the loader reads the system's Vulkan library, which is the
        // documented way to start.
        let entry = unsafe { ash::Entry::load() }
            .map_err(|e| fail(format!("no Vulkan loader: {e}")))?;

        let display = window
            .display_handle()
            .map_err(|e| fail(format!("no display handle: {e}")))?;
        let window_handle = window
            .window_handle()
            .map_err(|e| fail(format!("no window handle: {e}")))?;

        // 1.3, which the renderer needs to share the device
        let application = vk::ApplicationInfo::default()
            .application_name(c"Zakuro")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"Zakuro")
            .api_version(vk::API_VERSION_1_3);

        let extensions = ash_window::enumerate_required_extensions(display.as_raw())
            .map_err(vk_fail("querying surface extensions"))?
            .to_vec();

        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_extension_names(&extensions);
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(vk_fail("creating the instance"))?;

        let surface_instance = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(
                &entry,
                &instance,
                display.as_raw(),
                window_handle.as_raw(),
                None,
            )
        }
        .map_err(vk_fail("creating the surface"))?;

        let (physical_device, queue_family) =
            pick_device(&instance, &surface_instance, surface)?;

        let properties = unsafe { instance.get_physical_device_properties(physical_device) };
        let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) };

        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        // the renderer draws on this device too when it can, so the screens
        // it draws are shown from its images rather than copied out and up
        // again, ZAKURO_COPY_SCREENS keeps the copies
        let copy = std::env::var_os("ZAKURO_COPY_SCREENS").is_some();
        let swapchain_extension = [ash::khr::swapchain::NAME.as_ptr()];
        let shared = if copy || !can_render(&instance, physical_device, queue_family) {
            None
        } else {
            render_device(&entry, &instance, physical_device, queue_family, &swapchain_extension)
                .inspect_err(|error| log::warn!("could not make a device to share with the renderer, {error}"))
                .ok()
                .map(Arc::new)
        };
        // not destroyed on the way out of a failure below, which leaves what
        // was made from it, as everything else here does
        let shared = ManuallyDrop::new(shared);
        let device = match shared.as_deref() {
            Some(shared) => shared.device.clone(),
            None => {
                let priorities = [1.0f32];
                let queue_info = [vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(queue_family)
                    .queue_priorities(&priorities)];
                let device_info = vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_info)
                    .enabled_extension_names(&swapchain_extension);
                unsafe { instance.create_device(physical_device, &device_info, None) }
                    .map_err(vk_fail("creating the device"))?
            }
        };
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        let sharing = if shared.is_some() { ", the renderer's device too" } else { "" };
        log::info!("Vulkan on {}{sharing}", name.to_string_lossy());

        let swapchain_device = ash::khr::swapchain::Device::new(&instance, &device);
        let surface_format = choose_format(&surface_instance, physical_device, surface)?;
        let render_pass = create_render_pass(&device, surface_format.format)?;

        let descriptor_layout = create_descriptor_layout(&device, 1)?;
        let (pipeline_layout, pipeline) =
            create_pipeline(&device, render_pass, &[descriptor_layout], FRAGMENT_SPIRV, CONSTANTS)?;
        let stereo_layout = create_descriptor_layout(&device, 2)?;
        let (stereo_pipeline_layout, stereo_pipeline) =
            create_pipeline(&device, render_pass, &[stereo_layout], STEREO_FRAGMENT_SPIRV, STEREO_CONSTANTS)?;
        let srgb = matches!(surface_format.format, vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB);
        let overlay = OverlayPainter::new(&device, render_pass, FRAMES_IN_FLIGHT, srgb)?;

        let sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    // linear, the screens come drawn at up to four times the
                    // console's resolution and get shrunk as often as grown
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        }
        .map_err(vk_fail("creating the sampler"))?;

        let descriptor_pool = unsafe {
            // a screen whose size changes is made again before the old one
            // goes and gives its set back, so there is room for both, then
            // the sets for the renderer's images and the 3D sets, which hold
            // two pictures each, then the FSR filter's: SMAA's three sets
            // for each screen, five pictures, made after the old ones go,
            // and the upscaled screens' one
            let sets = 6 + 4 * FRAMES_IN_FLIGHT as u32 + 3 * 3 + 1;
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(6 + 5 * FRAMES_IN_FLIGHT as u32 + 3 * 5 + 1)];
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .pool_sizes(&sizes)
                    .max_sets(sets),
                None,
            )
        }
        .map_err(vk_fail("creating the descriptor pool"))?;

        let gpu_sets = unsafe {
            let layouts = [descriptor_layout; 3 * FRAMES_IN_FLIGHT];
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(vk_fail("allocating descriptor sets"))?
        .chunks(3)
        .map(|sets| [sets[0], sets[1], sets[2]])
        .collect();
        let stereo_sets = unsafe {
            let layouts = [stereo_layout; FRAMES_IN_FLIGHT];
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(vk_fail("allocating descriptor sets"))?;

        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }
        .map_err(vk_fail("creating the command pool"))?;

        let command_buffers = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(FRAMES_IN_FLIGHT as u32),
            )
        }
        .map_err(vk_fail("allocating command buffers"))?;

        let fsr = Fsr::new(&device, &memory_properties, queue, command_pool, render_pass, descriptor_layout)
            .inspect_err(|error| log::warn!("the FSR filter will upscale without SMAA or RCAS, {error}"))
            .ok();

        let mut image_available = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut render_finished = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut in_flight = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            let semaphore_info = vk::SemaphoreCreateInfo::default();
            image_available.push(
                unsafe { device.create_semaphore(&semaphore_info, None) }
                    .map_err(vk_fail("creating a semaphore"))?,
            );
            render_finished.push(
                unsafe { device.create_semaphore(&semaphore_info, None) }
                    .map_err(vk_fail("creating a semaphore"))?,
            );
            in_flight.push(
                unsafe {
                    device.create_fence(
                        // signalled, so the first frame does not wait forever.
                        &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                        None,
                    )
                }
                .map_err(vk_fail("creating a fence"))?,
            );
        }

        // the screens start at their native sizes, upload grows them if a
        // title ever reports something different.
        let screens = [
            Screen::new(
                &device,
                &memory_properties,
                descriptor_pool,
                descriptor_layout,
                sampler,
                400,
                240,
            )?,
            Screen::new(
                &device,
                &memory_properties,
                descriptor_pool,
                descriptor_layout,
                sampler,
                320,
                240,
            )?,
            Screen::new(
                &device,
                &memory_properties,
                descriptor_pool,
                descriptor_layout,
                sampler,
                400,
                240,
            )?,
        ];

        let mut presenter = VulkanPresenter {
            _entry: entry,
            instance,
            surface_instance,
            surface,
            physical_device,
            memory_properties,
            device,
            queue_family,
            queue,
            swapchain_device,
            swapchain: vk::SwapchainKHR::null(),
            surface_format,
            extent: vk::Extent2D {
                width: size.0,
                height: size.1,
            },
            image_views: Vec::new(),
            framebuffers: Vec::new(),
            render_pass,
            descriptor_layout,
            descriptor_pool,
            pipeline_layout,
            pipeline,
            stereo_layout,
            stereo_pipeline_layout,
            stereo_pipeline,
            stereo_sets,
            sampler,
            gpu_sets,
            shared: ManuallyDrop::into_inner(shared),
            waits: true,
            command_pool,
            command_buffers,
            image_available,
            render_finished,
            in_flight,
            frame: 0,
            screens,
            overlay: Some(overlay),
            fsr,
            window: size,
            arrangement: ScreenLayout::default(),
            top_width: super::TOP_WIDTH,
            filter: ScreenFilter::default(),
            integer: false,
            stereo: Stereo::default(),
            stale: false,
        };

        presenter.build_swapchain()?;
        Ok(presenter)
    }

    fn build_swapchain(&mut self) -> Result<(), PresentError> {
        let capabilities = unsafe {
            self.surface_instance
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)
        }
        .map_err(vk_fail("querying surface capabilities"))?;

        // a current extent of u32::MAX means the surface takes its size from
        // the swapchain rather than the other way round.
        let extent = if capabilities.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: self.window.0.clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: self.window.1.clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        } else {
            capabilities.current_extent
        };

        if extent.width == 0 || extent.height == 0 {
            // the window is minimized, nothing to build.
            self.extent = extent;
            return Ok(());
        }

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        let present_modes = unsafe {
            self.surface_instance
                .get_physical_device_surface_present_modes(self.physical_device, self.surface)
        }
        .map_err(vk_fail("querying present modes"))?;
        // mailbox where there is one, the frontend keeps the frames to the
        // console's rate itself, and a present that waits for the display,
        // as FIFO does on Mesa under Wayland, holds the next frame back with
        // it. FIFO is always there otherwise. ZAKURO_PRESENT_MODE=fifo,
        // mailbox or immediate picks one
        let wanted = match std::env::var("ZAKURO_PRESENT_MODE").as_deref() {
            Ok("fifo") => vec![vk::PresentModeKHR::FIFO],
            Ok("immediate") => vec![vk::PresentModeKHR::IMMEDIATE],
            Ok("mailbox") => vec![vk::PresentModeKHR::MAILBOX],
            _ => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO],
        };
        let present_mode = wanted.into_iter().find(|mode| present_modes.contains(mode)).unwrap_or(vk::PresentModeKHR::FIFO);
        self.waits = present_mode == vk::PresentModeKHR::FIFO;
        if self.swapchain == vk::SwapchainKHR::null() {
            let name = |mode: vk::PresentModeKHR| match mode {
                vk::PresentModeKHR::FIFO => "fifo".to_owned(),
                vk::PresentModeKHR::MAILBOX => "mailbox".to_owned(),
                vk::PresentModeKHR::IMMEDIATE => "immediate".to_owned(),
                vk::PresentModeKHR::FIFO_RELAXED => "fifo relaxed".to_owned(),
                other => format!("mode {}", other.as_raw()),
            };
            let all: Vec<String> = present_modes.iter().copied().map(name).collect();
            log::info!("presenting in {} mode, of {}", name(present_mode), all.join(", "));
        }

        let old = self.swapchain;
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(self.surface_format.format)
            .image_color_space(self.surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old);

        let swapchain = unsafe { self.swapchain_device.create_swapchain(&info, None) }
            .map_err(vk_fail("creating the swapchain"))?;

        self.destroy_swapchain_resources();
        if old != vk::SwapchainKHR::null() {
            unsafe { self.swapchain_device.destroy_swapchain(old, None) };
        }

        self.swapchain = swapchain;
        self.extent = extent;

        let images = unsafe { self.swapchain_device.get_swapchain_images(swapchain) }
            .map_err(vk_fail("getting swapchain images"))?;

        for image in images {
            let view = unsafe {
                self.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(self.surface_format.format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )
            }
            .map_err(vk_fail("creating a swapchain image view"))?;

            let attachments = [view];
            let framebuffer = unsafe {
                self.device.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(self.render_pass)
                        .attachments(&attachments)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1),
                    None,
                )
            }
            .map_err(vk_fail("creating a framebuffer"))?;

            self.image_views.push(view);
            self.framebuffers.push(framebuffer);
        }

        self.stale = false;
        Ok(())
    }

    fn destroy_swapchain_resources(&mut self) {
        unsafe {
            for framebuffer in self.framebuffers.drain(..) {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            for view in self.image_views.drain(..) {
                self.device.destroy_image_view(view, None);
            }
        }
        // the upscaled screens are the window's size, made again when next
        // wanted
        if let Some(fsr) = &mut self.fsr {
            fsr.drop_upscaled(&self.device, self.descriptor_pool);
        }
    }

    /// the frame's places, each marked with how it is drawn, see through.
    fn mark_sharpened(&self, places: Vec<(Viewport, Shown)>, sizes: &[(u32, u32); 3]) -> Vec<Place> {
        let usable = self.filter == ScreenFilter::Fsr && self.fsr.as_ref().is_some_and(|fsr| !fsr.broken);
        places
            .into_iter()
            .map(|(viewport, shown)| (viewport, shown, through(usable, shown, viewport, sizes[screen_index(shown)])))
            .collect()
    }

    /// makes what the antialiased places need: SMAA's images for each of
    /// their screens at its size, and the image the sharpened ones are
    /// upscaled into.
    fn prepare_fsr(&mut self, places: &[Place], sizes: &[(u32, u32); 3]) -> Result<(), PresentError> {
        let Some(fsr) = &mut self.fsr else { return Ok(()) };
        let mut wanted = [false; 3];
        for &(_, shown, through) in places {
            wanted[screen_index(shown)] |= through != Through::Plain;
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
            let smaa = fsr.smaa(&self.device, &self.memory_properties, self.descriptor_pool, self.descriptor_layout, self.sampler, size)?;
            fsr.screens[index] = Some(smaa);
        }
        if places.iter().any(|place| place.2 == Through::Sharpened) && fsr.upscaled.is_none() {
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

    /// copies pixels into a screen's staging buffer, reallocating the image if
    /// the size changed.
    fn upload(&mut self, index: usize, image: &ScreenImage<'_>) -> Result<(), PresentError> {
        if self.screens[index].width != image.width || self.screens[index].height != image.height
        {
            unsafe { self.device.device_wait_idle() }.map_err(vk_fail("waiting for idle"))?;
            let old = std::mem::replace(
                &mut self.screens[index],
                Screen::new(
                    &self.device,
                    &self.memory_properties,
                    self.descriptor_pool,
                    self.descriptor_layout,
                    self.sampler,
                    image.width,
                    image.height,
                )?,
            );
            old.destroy(&self.device, self.descriptor_pool);
        }

        let screen = &mut self.screens[index];
        let bytes = (image.width * image.height * 4) as usize;
        let count = bytes.min(image.pixels.len());
        // SAFETY: the staging buffer is host visible, persistently mapped and
        // at least bytes long, and no command using it is in flight because
        // the frame's fence was waited on first.
        unsafe {
            std::ptr::copy_nonoverlapping(image.pixels.as_ptr(), screen.staging_mapping, count);
        }
        screen.dirty = true;
        screen.showing = true;
        Ok(())
    }

    /// the device, for the renderer to draw on, when it can share it.
    pub fn shared_device(&self) -> Option<Arc<SharedDevice>> {
        self.shared.clone()
    }

    /// whether a present waits for the display, rather than handing the
    /// frame over and going on.
    pub fn waits_for_display(&self) -> bool {
        self.waits
    }

    /// what a screen is drawn from this frame, pixels uploaded to its own
    /// image or an image of the renderer's the frame's set points at.
    fn source(&mut self, index: usize, image: &ScreenImage<'_>) -> Result<Source, PresentError> {
        if let Some(gpu) = image.gpu {
            self.screens[index].showing = false;
            let info = [vk::DescriptorImageInfo::default()
                .sampler(self.sampler)
                .image_view(gpu.view)
                .image_layout(vk::ImageLayout::GENERAL)];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(self.gpu_sets[self.frame][index])
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&info);
            // SAFETY: the frame's fence was waited on, so no command using
            // the set is in flight
            unsafe { self.device.update_descriptor_sets(&[write], &[]) };
            return Ok(Source::Gpu(gpu));
        }
        if !image.is_empty() {
            self.upload(index, image)?;
            return Ok(Source::Own);
        }
        let screen = &self.screens[index];
        Ok(if screen.initialized && screen.showing { Source::Own } else { Source::Nothing })
    }

    /// points the frame's 3D set at the top screen's two pictures, wherever
    /// each is drawn from.
    fn write_stereo_set(&self, sources: &[Source; 3]) {
        let view = |index: usize| match sources[index] {
            Source::Gpu(gpu) => Some((gpu.view, vk::ImageLayout::GENERAL)),
            Source::Own => Some((self.screens[index].view, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)),
            Source::Nothing => None,
        };
        let (Some(left), Some(right)) = (view(0), view(2)) else { return };
        let infos = [left, right]
            .map(|(view, layout)| [vk::DescriptorImageInfo::default().sampler(self.sampler).image_view(view).image_layout(layout)]);
        let writes = [0, 1].map(|binding| {
            vk::WriteDescriptorSet::default()
                .dst_set(self.stereo_sets[self.frame])
                .dst_binding(binding as u32)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&infos[binding])
        });
        // SAFETY: the frame's fence was waited on, so no command using the
        // set is in flight
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
    }

    /// keeps the overlay's changes for the next frame drawn.
    fn defer(&mut self, overlay: &Overlay) {
        if let Some(painter) = &mut self.overlay {
            painter.defer(overlay);
        }
    }

    fn record(
        &self,
        command_buffer: vk::CommandBuffer,
        framebuffer: vk::Framebuffer,
        painter: &mut OverlayPainter,
        overlay: &Overlay,
        sources: [Source; 3],
        places: &[Place],
    ) -> Result<(), PresentError> {
        let device = &self.device;
        unsafe {
            device
                .begin_command_buffer(
                    command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .expect("begin command buffer");
        }
        painter.upload(device, &self.memory_properties, command_buffer, self.frame, overlay)?;
        unsafe {

            for screen in &self.screens {
                if !screen.dirty {
                    continue;
                }
                let old_layout = if screen.initialized {
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                } else {
                    vk::ImageLayout::UNDEFINED
                };
                transition(
                    device,
                    command_buffer,
                    screen.image,
                    old_layout,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                );

                device.cmd_copy_buffer_to_image(
                    command_buffer,
                    screen.staging,
                    screen.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(vk::Extent3D {
                            width: screen.width,
                            height: screen.height,
                            depth: 1,
                        })],
                );

                transition(
                    device,
                    command_buffer,
                    screen.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                );
            }

            // the renderer's work came first on the queue, its images are
            // read once what it wrote is there
            if sources.iter().any(|source| matches!(source, Source::Gpu(_))) {
                let barrier = vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ);
                device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[barrier],
                    &[],
                    &[],
                );
            }

            // each screen's set and place in its image
            let crops = [0, 1, 2].map(|index| match sources[index] {
                Source::Nothing => None,
                Source::Own => Some((self.screens[index].descriptor, WHOLE)),
                Source::Gpu(gpu) => {
                    let [x, y, width, height] = gpu.area;
                    let [left, top, right, bottom] = gpu.bounds;
                    Some((self.gpu_sets[self.frame][index], [x, y, width, height, left, top, right, bottom]))
                }
            });
            // the FSR filter's screens antialiased, and upscaled before the
            // window's pass sharpens them
            if let Some(fsr) = self.fsr.as_ref().filter(|_| places.iter().any(|place| place.2 != Through::Plain)) {
                fsr.record(device, command_buffer, places, &crops, self.extent);
            }

            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            }];
            device.cmd_begin_render_pass(
                command_buffer,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(framebuffer)
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D { x: 0, y: 0 },
                        extent: self.extent,
                    })
                    .clear_values(&clear),
                vk::SubpassContents::INLINE,
            );
            for &(viewport, shown, through) in places {
                if through == Through::Sharpened {
                    if let Some(fsr) = &self.fsr {
                        fsr.sharpen(device, command_buffer, viewport, self.extent);
                    }
                    continue;
                }
                // a screen through SMAA alone is drawn from SMAA's result,
                // which holds just the screen, smoothly
                let antialiased = self.fsr.as_ref().and_then(|fsr| fsr.screens[screen_index(shown)].as_ref()).filter(|_| through == Through::Smaa);
                let Some((descriptor, area)) = (match antialiased {
                    Some(smaa) => Some((smaa.result_input, WHOLE)),
                    None => crops[screen_index(shown)],
                }) else {
                    continue;
                };
                let filter = if antialiased.is_some() { ScreenFilter::Smooth } else { self.filter };
                let mut constants: Vec<u8> = area.iter().flat_map(|c| c.to_le_bytes()).collect();
                constants.extend_from_slice(&filter.mode().to_le_bytes());
                let (pipeline, layout, set) = if shown == Shown::Both {
                    // then how the eyes show, and the right eye's place
                    let Some((_, right)) = crops[2] else { continue };
                    let stereo = if self.stereo.mode == StereoMode::Anaglyph { 1i32 } else { 2 };
                    for word in [stereo, self.stereo.swap as i32, 0] {
                        constants.extend_from_slice(&word.to_le_bytes());
                    }
                    constants.extend(right.iter().flat_map(|c| c.to_le_bytes()));
                    (self.stereo_pipeline, self.stereo_pipeline_layout, self.stereo_sets[self.frame])
                } else {
                    (self.pipeline, self.pipeline_layout, descriptor)
                };
                set_viewport(device, command_buffer, viewport, self.extent);
                device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
                device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[set], &[]);
                device.cmd_push_constants(command_buffer, layout, vk::ShaderStageFlags::FRAGMENT, 0, &constants);
                device.cmd_draw(command_buffer, 3, 1, 0, 0);
            }

            painter.draw(device, command_buffer, self.frame, overlay, self.extent);
            device.cmd_end_render_pass(command_buffer);
            device.end_command_buffer(command_buffer).expect("end command buffer");
        }
        Ok(())
    }
}

impl Presenter for VulkanPresenter {
    fn name(&self) -> &'static str {
        "vulkan"
    }

    fn present(
        &mut self,
        top: ScreenImage<'_>,
        bottom: ScreenImage<'_>,
        right: ScreenImage<'_>,
        overlay: &Overlay,
    ) -> Result<(), PresentError> {
        if self.stale {
            // the swapchain no longer matches the window. rebuilding it is
            // what lets a minimized window come back: while minimized the
            // surface has no size, nothing is built, and stale stays set
            // until the window is restored to a size again, so the guard
            // below for an empty extent must not come first.
            self.defer(overlay);
            unsafe { self.device.device_wait_idle() }.map_err(vk_fail("waiting for idle"))?;
            self.build_swapchain()?;
            if self.extent.width == 0 || self.extent.height == 0 {
                // still minimized, keep waiting for a size.
                return Ok(());
            }
            return Err(PresentError::OutOfDate);
        }
        if self.extent.width == 0 || self.extent.height == 0 {
            self.defer(overlay);
            return Ok(());
        }

        let frame = self.frame;
        let fence = self.in_flight[frame];
        // with RUST_LOG=zakuro_gpu::present=debug, where showing a frame
        // spends its time: the fence, the image, recording, presenting
        let started = std::time::Instant::now();
        unsafe {
            self.device
                .wait_for_fences(&[fence], true, u64::MAX)
                .map_err(vk_fail("waiting for a fence"))?;
        }

        let fenced = started.elapsed();
        let acquired = unsafe {
            self.swapchain_device.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available[frame],
                vk::Fence::null(),
            )
        };
        let (image_index, suboptimal) = match acquired {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                self.defer(overlay);
                return Err(PresentError::OutOfDate);
            }
            Err(error) => {
                self.defer(overlay);
                return Err(vk_fail("acquiring an image")(error));
            }
        };

        let got = started.elapsed();
        // only reset the fence once we know we are going to submit.
        unsafe {
            self.device
                .reset_fences(&[fence])
                .map_err(vk_fail("resetting a fence"))?;
        }

        for screen in &mut self.screens {
            screen.dirty = false;
        }
        // the right eye's picture shows only in a frame that has one
        let has_right = !right.is_empty();
        let sources = [self.source(0, &top)?, self.source(1, &bottom)?, self.source(2, &right)?];
        if has_right && matches!(self.stereo.mode, StereoMode::Anaglyph | StereoMode::Interlaced) {
            self.write_stereo_set(&sources);
        }
        // each screen's pixels, an image of the renderer's holding the
        // screen at the size the frontend gives
        let images = [&top, &bottom, &right];
        let sizes = [0, 1, 2].map(|index| match sources[index] {
            Source::Nothing => (0, 0),
            Source::Own => (self.screens[index].width, self.screens[index].height),
            Source::Gpu(_) => (images[index].width, images[index].height),
        });
        let places = place(self.extent.width, self.extent.height, self.arrangement, self.top_width, self.integer, self.stereo, has_right);
        let mut places = self.mark_sharpened(places, &sizes);
        if places.iter().any(|place| place.2 != Through::Plain) {
            if let Err(error) = self.prepare_fsr(&places, &sizes) {
                // the fence was reset, so the frame is drawn anyway, with
                // EASU alone from now on
                log::warn!("the FSR filter will upscale without SMAA or RCAS, {error}");
                if let Some(fsr) = &mut self.fsr {
                    fsr.broken = true;
                }
                for place in &mut places {
                    place.2 = Through::Plain;
                }
            }
        }

        let command_buffer = self.command_buffers[frame];
        unsafe {
            self.device
                .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(vk_fail("resetting a command buffer"))?;
        }
        // the painter is taken out while it records, it and the presenter
        // both being borrowed
        let mut painter = self.overlay.take().expect("the overlay painter lives as long as the presenter");
        painter.begin_frame(&self.device, frame);
        let recorded = self.record(command_buffer, self.framebuffers[image_index as usize], &mut painter, overlay, sources, &places);
        painter.free(frame, overlay);
        self.overlay = Some(painter);
        recorded?;

        for screen in &mut self.screens {
            if screen.dirty {
                screen.initialized = true;
            }
        }

        let wait = [self.image_available[frame]];
        let signal = [self.render_finished[frame]];
        let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let buffers = [command_buffer];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&wait)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&buffers)
            .signal_semaphores(&signal);
        unsafe {
            self.device
                .queue_submit(self.queue, &[submit], fence)
                .map_err(vk_fail("submitting"))?;
        }

        let submitted = started.elapsed();
        let swapchains = [self.swapchain];
        let indices = [image_index];
        let present = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal)
            .swapchains(&swapchains)
            .image_indices(&indices);
        let result = unsafe { self.swapchain_device.queue_present(self.queue, &present) };
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        log::debug!(
            target: "zakuro_gpu::present",
            "fence {:.1} ms, image {:.1} ms, recording {:.1} ms, presenting {:.1} ms",
            ms(fenced),
            ms(got - fenced),
            ms(submitted - got),
            ms(started.elapsed() - submitted)
        );

        self.frame = (self.frame + 1) % FRAMES_IN_FLIGHT;

        match result {
            Ok(false) if !suboptimal => Ok(()),
            Ok(_) => {
                self.stale = true;
                Ok(())
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                Err(PresentError::OutOfDate)
            }
            Err(error) => Err(vk_fail("presenting")(error)),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.window = (width.max(1), height.max(1));
        self.stale = true;
    }

    fn set_layout(&mut self, arrangement: ScreenLayout) {
        self.arrangement = arrangement;
    }

    fn set_top_width(&mut self, width: f32) {
        self.top_width = width;
    }

    fn set_scaling(&mut self, filter: ScreenFilter, integer: bool) {
        self.filter = filter;
        self.integer = integer;
    }

    fn set_stereo(&mut self, stereo: Stereo) {
        self.stereo = stereo;
    }
}

impl Drop for VulkanPresenter {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            for screen in std::mem::replace(&mut self.screens, [Screen::null(), Screen::null(), Screen::null()]) {
                screen.destroy(&self.device, self.descriptor_pool);
            }
            self.destroy_swapchain_resources();
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_device.destroy_swapchain(self.swapchain, None);
            }
            for semaphore in self.image_available.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            for semaphore in self.render_finished.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            for fence in self.in_flight.drain(..) {
                self.device.destroy_fence(fence, None);
            }
            if let Some(mut painter) = self.overlay.take() {
                painter.destroy(&self.device);
            }
            if let Some(fsr) = self.fsr.take() {
                fsr.destroy(&self.device, self.descriptor_pool);
            }
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_pipeline(self.stereo_pipeline, None);
            self.device.destroy_pipeline_layout(self.stereo_pipeline_layout, None);
            self.device.destroy_descriptor_set_layout(self.stereo_layout, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_layout, None);
            self.device.destroy_render_pass(self.render_pass, None);
            // a shared device and its instance go with the last of the
            // renderer and this, the surface before
            if self.shared.is_none() {
                self.device.destroy_device(None);
            }
            self.surface_instance.destroy_surface(self.surface, None);
            if self.shared.is_none() {
                self.instance.destroy_instance(None);
            }
        }
        let _ = self.queue_family;
    }
}

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

fn pick_device(
    instance: &ash::Instance,
    surface_instance: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32), PresentError> {
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(vk_fail("enumerating devices"))?;

    let mut best: Option<(vk::PhysicalDevice, u32, u32)> = None;
    for device in devices {
        let families = unsafe { instance.get_physical_device_queue_family_properties(device) };
        for (index, family) in families.iter().enumerate() {
            if !family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                continue;
            }
            let supported = unsafe {
                surface_instance.get_physical_device_surface_support(
                    device,
                    index as u32,
                    surface,
                )
            }
            .unwrap_or(false);
            if !supported {
                continue;
            }
            // prefer a discrete GPU, then integrated, then anything.
            let properties = unsafe { instance.get_physical_device_properties(device) };
            let score = match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 3,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
                _ => 1,
            };
            if best.is_none_or(|(_, _, current)| score > current) {
                best = Some((device, index as u32, score));
            }
            break;
        }
    }

    best.map(|(device, family, _)| (device, family))
        .ok_or_else(|| fail("no Vulkan device can present to this window"))
}

fn choose_format(
    surface_instance: &ash::khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<vk::SurfaceFormatKHR, PresentError> {
    let formats = unsafe {
        surface_instance.get_physical_device_surface_formats(physical_device, surface)
    }
    .map_err(vk_fail("querying surface formats"))?;

    // the emulator produces straight sRGB-ish bytes, so an UNORM target keeps
    // them looking the way the console does.
    formats
        .iter()
        .copied()
        .find(|f| {
            f.format == vk::Format::B8G8R8A8_UNORM
                && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .or_else(|| formats.first().copied())
        .ok_or_else(|| fail("the surface reports no formats"))
}

fn create_render_pass(
    device: &ash::Device,
    format: vk::Format,
) -> Result<vk::RenderPass, PresentError> {
    let attachments = [vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];

    let color = [vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color)];

    let dependencies = [vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];

    unsafe {
        device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&dependencies),
            None,
        )
    }
    .map_err(vk_fail("creating the render pass"))
}

/// a set of pictures the fragment shader samples, one or the top screen's
/// two for 3D.
fn create_descriptor_layout(
    device: &ash::Device,
    pictures: u32,
) -> Result<vk::DescriptorSetLayout, PresentError> {
    let bindings: Vec<vk::DescriptorSetLayoutBinding> = (0..pictures)
        .map(|binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        })
        .collect();
    unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )
    }
    .map_err(vk_fail("creating the descriptor set layout"))
}

/// a pipeline drawing the vertex shader's triangle over the viewport with a
/// fragment shader, into the render pass's one picture, with sets of these
/// layouts, set 0 first.
fn create_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    layouts: &[vk::DescriptorSetLayout],
    fragment_spirv: &[u8],
    constants_size: u32,
) -> Result<(vk::PipelineLayout, vk::Pipeline), PresentError> {
    let vertex = create_shader_module(device, VERTEX_SPIRV)?;
    let fragment = create_shader_module(device, fragment_spirv)?;

    let constants = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        .size(constants_size)];
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(layouts)
                .push_constant_ranges(&constants),
            None,
        )
    }
    .map_err(vk_fail("creating the pipeline layout"))?;

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment)
            .name(c"main"),
    ];

    // the vertex shader builds its own triangle, so there is no input state.
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)];
    let blend =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterization)
        .multisample_state(&multisample)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipelines = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
    }
    .map_err(|(_, error)| vk_fail("creating the pipeline")(error))?;

    unsafe {
        device.destroy_shader_module(vertex, None);
        device.destroy_shader_module(fragment, None);
    }

    Ok((pipeline_layout, pipelines[0]))
}

pub(super) fn create_shader_module(
    device: &ash::Device,
    spirv: &[u8],
) -> Result<vk::ShaderModule, PresentError> {
    if !spirv.len().is_multiple_of(4) {
        return Err(fail("SPIR-V is not a whole number of words"));
    }
    let words: Vec<u32> = spirv
        .as_chunks::<4>().0.iter()
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    unsafe {
        device.create_shader_module(
            &vk::ShaderModuleCreateInfo::default().code(&words),
            None,
        )
    }
    .map_err(vk_fail("creating a shader module"))
}

pub(super) fn find_memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    wanted: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..properties.memory_type_count).find(|&i| {
        type_bits & (1 << i) != 0
            && properties.memory_types[i as usize]
                .property_flags
                .contains(wanted)
    })
}

pub(super) fn transition(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    image: vk::Image,
    from: vk::ImageLayout,
    to: vk::ImageLayout,
) {
    let (src_access, dst_access, src_stage, dst_stage) = match (from, to) {
        (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        ),
        (vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
            vk::AccessFlags::SHADER_READ,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::TRANSFER,
        ),
        _ => (
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
    };

    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(from)
        .new_layout(to)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
        )
        .src_access_mask(src_access)
        .dst_access_mask(dst_access);

    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}

fn set_viewport(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    viewport: Viewport,
    extent: vk::Extent2D,
) {
    unsafe {
        device.cmd_set_viewport(
            command_buffer,
            0,
            &[vk::Viewport {
                x: viewport.x,
                y: viewport.y,
                width: viewport.width,
                height: viewport.height,
                min_depth: 0.0,
                max_depth: 1.0,
            }],
        );
        // the scissor has to be clamped to the framebuffer or validation
        // complains when the window is smaller than the layout wants.
        let x = viewport.x.max(0.0) as i32;
        let y = viewport.y.max(0.0) as i32;
        let width = (viewport.width as u32).min(extent.width.saturating_sub(x as u32));
        let height = (viewport.height as u32).min(extent.height.saturating_sub(y as u32));
        device.cmd_set_scissor(
            command_buffer,
            0,
            &[vk::Rect2D {
                offset: vk::Offset2D { x, y },
                extent: vk::Extent2D { width, height },
            }],
        );
    }
}

impl Screen {
    fn null() -> Screen {
        Screen {
            width: 0,
            height: 0,
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            staging: vk::Buffer::null(),
            staging_memory: vk::DeviceMemory::null(),
            staging_mapping: std::ptr::null_mut(),
            staging_size: 0,
            descriptor: vk::DescriptorSet::null(),
            initialized: false,
            dirty: false,
            showing: false,
        }
    }

    fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        pool: vk::DescriptorPool,
        layout: vk::DescriptorSetLayout,
        sampler: vk::Sampler,
        width: u32,
        height: u32,
    ) -> Result<Screen, PresentError> {
        let size = (width as vk::DeviceSize) * (height as vk::DeviceSize) * 4;

        let image = unsafe {
            device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
        }
        .map_err(vk_fail("creating a screen image"))?;

        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let type_index = find_memory_type(
            memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| fail("no device-local memory for a screen image"))?;
        let memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(type_index),
                None,
            )
        }
        .map_err(vk_fail("allocating image memory"))?;
        unsafe { device.bind_image_memory(image, memory, 0) }
            .map_err(vk_fail("binding image memory"))?;

        let view = unsafe {
            device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    ),
                None,
            )
        }
        .map_err(vk_fail("creating a screen image view"))?;

        let staging = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(vk_fail("creating a staging buffer"))?;

        let staging_requirements = unsafe { device.get_buffer_memory_requirements(staging) };
        let staging_type = find_memory_type(
            memory_properties,
            staging_requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| fail("no host-visible memory for a staging buffer"))?;
        let staging_memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(staging_requirements.size)
                    .memory_type_index(staging_type),
                None,
            )
        }
        .map_err(vk_fail("allocating staging memory"))?;
        unsafe { device.bind_buffer_memory(staging, staging_memory, 0) }
            .map_err(vk_fail("binding staging memory"))?;

        // mapped once and kept mapped, the buffer is written every frame.
        let staging_mapping = unsafe {
            device.map_memory(
                staging_memory,
                0,
                staging_requirements.size,
                vk::MemoryMapFlags::empty(),
            )
        }
        .map_err(vk_fail("mapping staging memory"))? as *mut u8;

        let layouts = [layout];
        let descriptors = unsafe {
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(vk_fail("allocating a descriptor set"))?;
        let descriptor = descriptors[0];

        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        unsafe {
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(descriptor)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&image_info)],
                &[],
            );
        }

        Ok(Screen {
            width,
            height,
            image,
            memory,
            view,
            staging,
            staging_memory,
            staging_mapping,
            staging_size: staging_requirements.size,
            descriptor,
            initialized: false,
            dirty: false,
            showing: false,
        })
    }

    fn destroy(self, device: &ash::Device, pool: vk::DescriptorPool) {
        if self.image == vk::Image::null() {
            return;
        }
        unsafe {
            let _ = device.free_descriptor_sets(pool, &[self.descriptor]);
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
            device.unmap_memory(self.staging_memory);
            device.destroy_buffer(self.staging, None);
            device.free_memory(self.staging_memory, None);
        }
        let _ = self.staging_size;
    }
}

// ---------------------------------------------------------------------------
// The FSR filter's antialiasing and sharpening
// ---------------------------------------------------------------------------

/// where a screen goes in the window, which picture it shows, and whether
/// the FSR filter antialiases and sharpens it there.
type Place = (Viewport, Shown, Through);

/// how a place's screen goes onto the window.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Through {
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
type Pipeline = (vk::PipelineLayout, vk::Pipeline);

/// the screen a place shows: the top one, also for its two pictures in one,
/// the bottom one, or the top one's picture for the right eye.
fn screen_index(shown: Shown) -> usize {
    match shown {
        Shown::Top | Shown::Both => 0,
        Shown::Bottom => 1,
        Shown::Right => 2,
    }
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

/// an image of the presenter's own on the device, with a view of it.
struct Texture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

impl Texture {
    fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        (width, height): (u32, u32),
        usage: vk::ImageUsageFlags,
    ) -> Result<Texture, PresentError> {
        let image = unsafe {
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
        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let type_index = find_memory_type(memory_properties, requirements.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
            .ok_or_else(|| fail("no device-local memory for an image"))?;
        let memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default().allocation_size(requirements.size).memory_type_index(type_index),
                None,
            )
        }
        .map_err(vk_fail("allocating image memory"))?;
        unsafe { device.bind_image_memory(image, memory, 0) }.map_err(vk_fail("binding image memory"))?;
        let view = unsafe {
            device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
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
        Ok(Texture { image, memory, view })
    }

    fn destroy(&self, device: &ash::Device) {
        unsafe {
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

/// an image one of the FSR filter's passes draws into and a later one
/// reads.
struct Target {
    texture: Texture,
    framebuffer: vk::Framebuffer,
}

impl Target {
    fn new(
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

    fn destroy(&self, device: &ash::Device) {
        unsafe { device.destroy_framebuffer(self.framebuffer, None) };
        self.texture.destroy(device);
    }
}

/// a set of the layout's with these pictures at bindings 0 on, read in the
/// shader read only layout through the sampler.
fn picture_set(
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
struct Smaa {
    size: (u32, u32),
    edges: Target,
    weights: Target,
    result: Target,
    /// the edges and the area and search tables, for the weights.
    weights_inputs: vk::DescriptorSet,
    /// the weights, for the blending.
    blend_input: vk::DescriptorSet,
    /// the antialiased screen, for EASU.
    result_input: vk::DescriptorSet,
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
struct Fsr {
    edges_pass: vk::RenderPass,
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
    screens: [Option<Smaa>; 3],
    /// the screens upscaled, the window's size, and the set reading them,
    /// made when first wanted and again with the swapchain.
    upscaled: Option<(Target, vk::DescriptorSet)>,
    /// set when making the images failed, after which the filter is EASU
    /// alone.
    broken: bool,
}

impl Fsr {
    fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        command_pool: vk::CommandPool,
        window_pass: vk::RenderPass,
        picture_layout: vk::DescriptorSetLayout,
    ) -> Result<Fsr, PresentError> {
        let edges_pass = create_target_pass(device, EDGES_FORMAT)?;
        let colour_pass = create_target_pass(device, COLOUR_FORMAT)?;
        let lookup_layout = create_descriptor_layout(device, 3)?;
        let edges = create_pipeline(device, edges_pass, &[picture_layout], SMAA_EDGES_SPIRV, SMAA_CONSTANTS)?;
        let weights = create_pipeline(device, colour_pass, &[lookup_layout], SMAA_WEIGHTS_SPIRV, SMAA_CONSTANTS)?;
        // the screen in set 0, as the first pass reads it, the weights in 1
        let blend = create_pipeline(device, colour_pass, &[picture_layout, picture_layout], SMAA_BLEND_SPIRV, SMAA_CONSTANTS)?;
        let easu = create_pipeline(device, colour_pass, &[picture_layout], FRAGMENT_SPIRV, CONSTANTS)?;
        let rcas = create_pipeline(device, window_pass, &[picture_layout], RCAS_SPIRV, RCAS_CONSTANTS)?;
        let (area, search) = upload_tables(device, memory_properties, queue, command_pool)?;
        Ok(Fsr {
            edges_pass,
            colour_pass,
            lookup_layout,
            edges,
            weights,
            blend,
            easu,
            rcas,
            area,
            search,
            screens: [None, None, None],
            upscaled: None,
            broken: false,
        })
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
        let edges = Target::new(device, memory_properties, self.edges_pass, EDGES_FORMAT, size)?;
        let weights = Target::new(device, memory_properties, self.colour_pass, COLOUR_FORMAT, size)?;
        let result = Target::new(device, memory_properties, self.colour_pass, COLOUR_FORMAT, size)?;
        let tables = [edges.texture.view, self.area.view, self.search.view];
        let weights_inputs = picture_set(device, pool, self.lookup_layout, sampler, &tables)?;
        let blend_input = picture_set(device, pool, picture_layout, sampler, &[weights.texture.view])?;
        let result_input = picture_set(device, pool, picture_layout, sampler, &[result.texture.view])?;
        Ok(Smaa { size, edges, weights, result, weights_inputs, blend_input, result_input })
    }

    fn drop_upscaled(&mut self, device: &ash::Device, pool: vk::DescriptorPool) {
        if let Some((target, set)) = self.upscaled.take() {
            unsafe {
                let _ = device.free_descriptor_sets(pool, &[set]);
            }
            target.destroy(device);
        }
    }

    /// records SMAA's passes over each antialiased place's screen, once
    /// however many places show it, then EASU's of every sharpened place
    /// into the upscaled image, in the window's places.
    fn record(
        &self,
        device: &ash::Device,
        command_buffer: vk::CommandBuffer,
        places: &[Place],
        crops: &[Option<(vk::DescriptorSet, [f32; 8])>; 3],
        extent: vk::Extent2D,
    ) {
        let mut done = [false; 3];
        for &(_, shown, through) in places {
            let index = screen_index(shown);
            if through == Through::Plain || std::mem::replace(&mut done[index], true) {
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
        }

        // the whole of each screen drawn bigger, through EASU
        let Some((upscaled, _)) = self.upscaled.as_ref().filter(|_| places.iter().any(|place| place.2 == Through::Sharpened)) else { return };
        let mut constants = float_bytes(&WHOLE);
        constants.extend_from_slice(&ScreenFilter::Fsr.mode().to_le_bytes());
        let (layout, pipeline) = self.easu;
        begin_pass(device, command_buffer, self.colour_pass, upscaled.framebuffer, extent);
        for &(viewport, shown, through) in places {
            let Some(smaa) = self.screens[screen_index(shown)].as_ref().filter(|_| through == Through::Sharpened) else { continue };
            set_viewport(device, command_buffer, viewport, extent);
            unsafe {
                device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
                device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[smaa.result_input], &[]);
                device.cmd_push_constants(command_buffer, layout, vk::ShaderStageFlags::FRAGMENT, 0, &constants);
                device.cmd_draw(command_buffer, 3, 1, 0, 0);
            }
        }
        unsafe { device.cmd_end_render_pass(command_buffer) };
    }

    /// draws a sharpened place onto the window from the upscaled image,
    /// within the window's render pass.
    fn sharpen(&self, device: &ash::Device, command_buffer: vk::CommandBuffer, viewport: Viewport, extent: vk::Extent2D) {
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

    fn destroy(mut self, device: &ash::Device, pool: vk::DescriptorPool) {
        for smaa in self.screens.iter_mut().filter_map(Option::take) {
            smaa.destroy(device, pool);
        }
        self.drop_upscaled(device, pool);
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
fn begin_pass(
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
fn create_target_pass(device: &ash::Device, format: vk::Format) -> Result<vk::RenderPass, PresentError> {
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

/// SMAA's area and search tables on the device, copied up once and waited
/// for.
fn upload_tables(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
) -> Result<(Texture, Texture), PresentError> {
    let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
    let area = Texture::new(device, memory_properties, vk::Format::R8G8_UNORM, SMAA_AREA_SIZE, usage)?;
    let search = Texture::new(device, memory_properties, vk::Format::R8_UNORM, SMAA_SEARCH_SIZE, usage)?;

    // both in one buffer, the search table after the area one
    let size = (SMAA_AREA.len() + SMAA_SEARCH.len()) as vk::DeviceSize;
    let staging = unsafe {
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
    let memory = unsafe {
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
    let copies = [(&area, SMAA_AREA_SIZE, 0), (&search, SMAA_SEARCH_SIZE, SMAA_AREA.len())];
    let uploaded = unsafe {
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
    unsafe {
        device.free_command_buffers(command_pool, &buffers);
        device.destroy_buffer(staging, None);
        device.free_memory(memory, None);
    }
    uploaded?;
    Ok((area, search))
}

#[cfg(test)]
mod tests {
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
        assert!((rcas_sharpness(RCAS_SHARPNESS_STOPS) - 0.707_107).abs() < 1e-5);
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

    /// a host visible buffer, mapped.
    fn host_buffer(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        size: usize,
        usage: vk::BufferUsageFlags,
    ) -> (vk::Buffer, vk::DeviceMemory, *mut u8) {
        unsafe {
            let info = vk::BufferCreateInfo::default().size(size as u64).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&info, None).unwrap();
            let requirements = device.get_buffer_memory_requirements(buffer);
            let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let index = find_memory_type(memory_properties, requirements.memory_type_bits, wanted).unwrap();
            let allocation = vk::MemoryAllocateInfo::default().allocation_size(requirements.size).memory_type_index(index);
            let memory = device.allocate_memory(&allocation, None).unwrap();
            device.bind_buffer_memory(buffer, memory, 0).unwrap();
            let mapping = device.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty()).unwrap() as *mut u8;
            (buffer, memory, mapping)
        }
    }

    fn free_buffer(device: &ash::Device, (buffer, memory, _): (vk::Buffer, vk::DeviceMemory, *mut u8)) {
        unsafe {
            device.unmap_memory(memory);
            device.destroy_buffer(buffer, None);
            device.free_memory(memory, None);
        }
    }

    /// a whole RGBA8 image of this size, copied to or from a buffer.
    fn whole((width, height): (u32, u32)) -> vk::BufferImageCopy {
        vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1))
            .image_extent(vk::Extent3D { width, height, depth: 1 })
    }

    /// records commands, runs them and waits for them.
    fn run(shared: &crate::SharedDevice, pool: vk::CommandPool, record: impl FnOnce(vk::CommandBuffer)) {
        let device = &shared.device;
        unsafe {
            let info = vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
            let buffers = device.allocate_command_buffers(&info).unwrap();
            let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(buffers[0], &begin).unwrap();
            record(buffers[0]);
            device.end_command_buffer(buffers[0]).unwrap();
            device.queue_submit(shared.queue, &[vk::SubmitInfo::default().command_buffers(&buffers)], vk::Fence::null()).unwrap();
            device.queue_wait_idle(shared.queue).unwrap();
            device.free_command_buffers(pool, &buffers);
        }
    }

    /// the RGBA8 pixels of an image a pass of the filter's drew.
    fn read_back(
        shared: &crate::SharedDevice,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        pool: vk::CommandPool,
        image: vk::Image,
        size: (u32, u32),
    ) -> Vec<u8> {
        let device = &shared.device;
        let bytes = (size.0 * size.1 * 4) as usize;
        let buffer = host_buffer(device, memory_properties, bytes, vk::BufferUsageFlags::TRANSFER_DST);
        run(shared, pool, |command_buffer| unsafe {
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
        free_buffer(device, buffer);
        pixels
    }

    /// on the GPU, when there is one: a screen of a hard, shallow line
    /// comes out of SMAA blended along the line alone, and out of EASU and
    /// RCAS twice its size with its flat parts kept.
    #[test]
    fn the_fsr_filter_antialiases_upscales_and_sharpens_a_screen() {
        let shared = match crate::SharedDevice::new() {
            Ok(shared) => shared,
            Err(error) => {
                eprintln!("skipped, {error}");
                return;
            }
        };
        let device = &shared.device;
        let memory_properties = unsafe { shared.instance.get_physical_device_memory_properties(shared.physical) };
        let pool = unsafe { device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(shared.family), None) }.unwrap();
        let picture_layout = create_descriptor_layout(device, 1).unwrap();
        let sizes = [vk::DescriptorPoolSize::default().ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(8)];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .pool_sizes(&sizes)
            .max_sets(6);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }.unwrap();
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
        let sampler = unsafe { device.create_sampler(&sampler_info, None) }.unwrap();
        // the window's pass, compatible with the swapchain's but read back
        let window_pass = create_target_pass(device, COLOUR_FORMAT).unwrap();
        let mut fsr = Fsr::new(device, &memory_properties, shared.queue, pool, window_pass, picture_layout).unwrap();

        // white over black, the line one row lower every 8 pixels
        let (width, height) = (64u32, 64u32);
        let edge = |x: u32| 20 + x / 8;
        let pixels: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| if y < edge(x) { [255; 4] } else { [0, 0, 0, 255] }))
            .collect();
        let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
        let screen = Texture::new(device, &memory_properties, COLOUR_FORMAT, (width, height), usage).unwrap();
        let staging = host_buffer(device, &memory_properties, pixels.len(), vk::BufferUsageFlags::TRANSFER_SRC);
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), staging.2, pixels.len()) };
        run(&shared, pool, |command_buffer| unsafe {
            transition(device, command_buffer, screen.image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
            let layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
            device.cmd_copy_buffer_to_image(command_buffer, staging.0, screen.image, layout, &[whole((width, height))]);
            transition(device, command_buffer, screen.image, layout, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        });
        free_buffer(device, staging);
        let screen_set = picture_set(device, descriptor_pool, picture_layout, sampler, &[screen.view]).unwrap();

        // drawn at twice its size
        let smaa = fsr.smaa(device, &memory_properties, descriptor_pool, picture_layout, sampler, (width, height)).unwrap();
        fsr.screens[0] = Some(smaa);
        let extent = vk::Extent2D { width: width * 2, height: height * 2 };
        let upscaled = Target::new(device, &memory_properties, fsr.colour_pass, COLOUR_FORMAT, (extent.width, extent.height)).unwrap();
        let upscaled_set = picture_set(device, descriptor_pool, picture_layout, sampler, &[upscaled.texture.view]).unwrap();
        fsr.upscaled = Some((upscaled, upscaled_set));
        let window = Target::new(device, &memory_properties, window_pass, COLOUR_FORMAT, (extent.width, extent.height)).unwrap();
        let viewport = Viewport { x: 0.0, y: 0.0, width: extent.width as f32, height: extent.height as f32 };
        assert!(magnified(viewport, (width, height)));
        let places = [(viewport, Shown::Top, Through::Sharpened)];
        let crops = [Some((screen_set, WHOLE)), None, None];
        run(&shared, pool, |command_buffer| {
            fsr.record(device, command_buffer, &places, &crops, extent);
            begin_pass(device, command_buffer, window_pass, window.framebuffer, extent);
            fsr.sharpen(device, command_buffer, viewport, extent);
            unsafe { device.cmd_end_render_pass(command_buffer) };
        });

        let result = fsr.screens[0].as_ref().unwrap().result.texture.image;
        let antialiased = read_back(&shared, &memory_properties, pool, result, (width, height));
        let shown = read_back(&shared, &memory_properties, pool, window.texture.image, (extent.width, extent.height));
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
            device.destroy_sampler(sampler, None);
            device.destroy_descriptor_pool(descriptor_pool, None);
            device.destroy_descriptor_set_layout(picture_layout, None);
            device.destroy_render_pass(window_pass, None);
            device.destroy_command_pool(pool, None);
        }
    }
}
