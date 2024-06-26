//! Swapchain utils.
//!
//! Original implementation by [Ralith]
//! (<https://github.com/MaikKlein/ash/pull/506>).
//!
//! [Ralith]: https://github.com/Ralith

use ash::{extensions::khr::Surface, prelude::VkResult, vk, Device};
use std::{collections::VecDeque, mem};

/// Manages synchronizing and rebuilding a Vulkan swapchain.
pub struct Swapchain {
  options: SwapchainOptions,

  frames: Vec<Frame>,
  frame_index: usize,

  surface: vk::SurfaceKHR,
  physical_device: vk::PhysicalDevice,
  swapchain_ext: ash::extensions::khr::Swapchain,
  handle: vk::SwapchainKHR,
  generation: u64,
  images: Vec<vk::Image>,
  extent: vk::Extent2D,
  format: vk::SurfaceFormatKHR,
  needs_rebuild: bool,

  old_swapchains: VecDeque<(vk::SwapchainKHR, u64)>,
}

impl Swapchain {
  /// Construct a new [`Swapchain`] for rendering at most `frames_in_flight`
  /// frames concurrently. `extent` should be the current dimensions of
  /// `surface`.
  pub fn new(
    options: SwapchainOptions,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: &Device,
    swapchain_loader: ash::extensions::khr::Swapchain,
    extent: vk::Extent2D,
  ) -> Self {
    Self {
      frames: (0..options.frames_in_flight)
        .map(|_| unsafe {
          Frame {
            complete: device
              .create_fence(
                &vk::FenceCreateInfo::builder().flags(vk::FenceCreateFlags::SIGNALED),
                None,
              )
              .unwrap(),
            acquire: device
              .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
              .unwrap(),
            generation: 0,
          }
        })
        .collect(),
      frame_index: 0,

      surface,
      physical_device,
      swapchain_ext: swapchain_loader,
      handle: vk::SwapchainKHR::null(),
      generation: 0,
      images: Vec::new(),
      extent,
      format: vk::SurfaceFormatKHR::default(),
      needs_rebuild: true,

      old_swapchains: VecDeque::new(),

      options,
    }
  }

  /// Destroy all swapchain resources. Must not be called while any frames are
  /// still in flight on the GPU.
  ///
  /// # Safety
  ///
  /// - `device` must match the `device` passed to [`Swapchain::new`].
  /// - Access to images obtained from [`images`](Self::images) must be
  ///   externally synchronized.
  #[inline]
  pub unsafe fn destroy(&self, device: &Device) {
    for frame in &self.frames {
      device.destroy_fence(frame.complete, None);
      device.destroy_semaphore(frame.acquire, None);
    }

    self.swapchain_ext.destroy_swapchain(self.handle, None);

    for &(swapchain, _) in &self.old_swapchains {
      self.swapchain_ext.destroy_swapchain(swapchain, None);
    }
  }

  /// Force the swapchain to be rebuilt on the next [`acquire`](Self::acquire)
  /// call, passing in the surface's current size.
  #[inline]
  pub fn update(&mut self, extent: vk::Extent2D) {
    self.extent = extent;
    self.needs_rebuild = true;
  }

  /// Maximum number of frames that may be concurrently rendered.
  #[inline]
  pub fn frames_in_flight(&self) -> usize {
    self.frames.len()
  }

  /// Latest set of swapchain images, keyed by [`AcquiredFrame::image_index`].
  #[inline]
  pub fn images(&self) -> &[vk::Image] {
    &self.images
  }

  /// Format of images in [`images`](Self::images), and the color space that
  /// will be used to present them.
  #[inline]
  pub fn format(&self) -> vk::SurfaceFormatKHR {
    self.format
  }

  /// Dimensions of images in [`images`](Self::images).
  #[inline]
  pub fn extent(&self) -> vk::Extent2D {
    self.extent
  }

  /// Acquire resources to render a frame.
  ///
  /// Returns [`vk::Result::ERROR_OUT_OF_DATE_KHR`] if and only if the
  /// configured format or present modes could not be satisfied.
  ///
  /// # Safety
  ///
  /// `device` must have been created from `instance` and must match the
  /// `device` passed to [`Swapchain::new`].
  pub unsafe fn acquire(
    &mut self,
    device: &Device,
    surface_loader: &Surface,
    timeout_ns: u64,
    enable_fences: bool,
  ) -> VkResult<AcquiredFrame> {
    let frame_index = self.frame_index;
    let next_frame_index = (self.frame_index + 1) % self.frames.len();
    let frame = &self.frames[frame_index];
    let acquire = frame.acquire;
    if enable_fences {
      device.wait_for_fences(&[frame.complete], true, timeout_ns)?;
    }

    // Destroy swapchains that are guaranteed not to be in use now that this frame
    // has finished
    while let Some(&(swapchain, generation)) = self.old_swapchains.front() {
      if self.frames[next_frame_index].generation == generation {
        break;
      }
      self.swapchain_ext.destroy_swapchain(swapchain, None);
      self.old_swapchains.pop_front();
    }

    loop {
      if !self.needs_rebuild {
        let acquire_next_image = self
          .swapchain_ext
          .acquire_next_image(self.handle, !0, acquire, vk::Fence::null());
        match acquire_next_image {
          Ok((index, suboptimal)) => {
            self.needs_rebuild = suboptimal;
            let invalidate_images = self.frames[frame_index].generation != self.generation;
            self.frames[frame_index].generation = self.generation;
            self.frame_index = next_frame_index;
            device.reset_fences(&[self.frames[frame_index].complete]).unwrap();
            return Ok(AcquiredFrame {
              image_index: index as usize,
              frame_index,
              ready: acquire,
              complete: self.frames[frame_index].complete,
              invalidate_images,
            });
          }
          Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {}
          Err(e) => return Err(e),
        }
      };
      self.needs_rebuild = true;

      // Rebuild swapchain
      let surface_capabilities =
        surface_loader.get_physical_device_surface_capabilities(self.physical_device, self.surface)?;

      self.extent = match surface_capabilities.current_extent.width {
        // If Vulkan doesn't know, the windowing system probably does. Known to apply at
        // least to Wayland.
        u32::MAX => vk::Extent2D {
          width: self.extent.width,
          height: self.extent.height,
        },
        _ => surface_capabilities.current_extent,
      };

      let pre_transform = if surface_capabilities
        .supported_transforms
        .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
      {
        vk::SurfaceTransformFlagsKHR::IDENTITY
      } else {
        surface_capabilities.current_transform
      };

      let present_modes =
        surface_loader.get_physical_device_surface_present_modes(self.physical_device, self.surface)?;
      let present_mode = match present_modes
        .iter()
        .filter_map(|&mode| {
          Some((
            mode,
            self
              .options
              .present_mode_preference
              .iter()
              .position(|&pref| pref == mode)?,
          ))
        })
        .min_by_key(|&(_, priority)| priority)
      {
        Some((mode, _)) => mode,
        None => return Err(vk::Result::ERROR_OUT_OF_DATE_KHR),
      };

      let desired_image_count = (surface_capabilities.min_image_count + 1).max(self.frames.len() as u32);
      let image_count = if surface_capabilities.max_image_count > 0 {
        surface_capabilities.max_image_count.min(desired_image_count)
      } else {
        desired_image_count
      };

      let surface_formats = surface_loader.get_physical_device_surface_formats(self.physical_device, self.surface)?;
      match surface_formats
        .iter()
        .filter_map(|&format| {
          Some((
            format,
            self.options.format_preference.iter().position(|&pref| pref == format)?,
          ))
        })
        .min_by_key(|&(_, priority)| priority)
      {
        Some((format, _)) => self.format = format,
        None => return Err(vk::Result::ERROR_OUT_OF_DATE_KHR),
      };

      if self.handle != vk::SwapchainKHR::null() {
        self.old_swapchains.push_back((self.handle, self.generation));
      }
      let handle = self.swapchain_ext.create_swapchain(
        &vk::SwapchainCreateInfoKHR::builder()
          .surface(self.surface)
          .min_image_count(image_count)
          .image_color_space(self.format.color_space)
          .image_format(self.format.format)
          .image_extent(self.extent)
          .image_usage(self.options.usage)
          .image_sharing_mode(self.options.sharing_mode)
          .pre_transform(pre_transform)
          .composite_alpha(self.options.composite_alpha)
          .present_mode(present_mode)
          .clipped(true)
          .image_array_layers(1)
          .old_swapchain(mem::replace(&mut self.handle, vk::SwapchainKHR::null())),
        None,
      )?;
      self.generation = self.generation.wrapping_add(1);
      self.handle = handle;
      self.images = self.swapchain_ext.get_swapchain_images(handle)?;
      self.needs_rebuild = false;
    }
  }

  /// Queue presentation of a previously acquired image.
  ///
  /// # Safety
  ///
  /// In addition to the usual requirements of [`Device::queue_present_khr`]:
  ///
  /// - `device` must match the `device` passed to [`Swapchain::new`].
  /// - `image_index` must have been obtained from an
  ///   [`AcquiredFrame::image_index`] from a previous
  ///   [`acquire`](Self::acquire) call which has not yet been passed to
  ///   queue_present.
  /// - A command buffer that will signal `render_complete` after finishing
  ///   access to the `image_index` element of [`images`](Self::images) must
  ///   have been submitted.
  #[inline]
  pub unsafe fn queue_present(
    &mut self,
    queue: vk::Queue,
    render_complete: vk::Semaphore,
    image_index: usize,
  ) -> VkResult<()> {
    let queue_present = self.swapchain_ext.queue_present(
      queue,
      &vk::PresentInfoKHR::builder()
        .wait_semaphores(&[render_complete])
        .swapchains(&[self.handle])
        .image_indices(&[image_index as u32]),
    );

    if let Err(vk::Result::SUBOPTIMAL_KHR) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) = queue_present {
      self.needs_rebuild = true;
      Ok(())
    } else {
      queue_present.map(|_| ())
    }
  }

  /// Get the raw swapchain handle for vkguide.dev compatibility as vk-bootstrap
  /// doesn't handle presentation on it's own
  pub fn handle(&self) -> vk::SwapchainKHR{
    self.handle
  }
}

/// [`Swapchain`] configuration.
#[derive(Debug, Clone)]
pub struct SwapchainOptions {
  frames_in_flight: usize,
  format_preference: Vec<vk::SurfaceFormatKHR>,
  present_mode_preference: Vec<vk::PresentModeKHR>,
  usage: vk::ImageUsageFlags,
  sharing_mode: vk::SharingMode,
  composite_alpha: vk::CompositeAlphaFlagsKHR,
}

impl SwapchainOptions {
  /// Uses the default values.
  #[inline]
  pub fn new() -> Self {
    Self::default()
  }

  /// Number of frames that may be concurrently worked on, including recording
  /// on the CPU. Defaults to 2.
  #[inline]
  pub fn frames_in_flight(mut self, frames: usize) -> Self {
    self.frames_in_flight = frames;
    self
  }

  /// Preference-ordered list of image formats and color spaces. Defaults to
  /// 8-bit sRGB.
  #[inline]
  pub fn format_preference(mut self, formats: &[vk::SurfaceFormatKHR]) -> Self {
    self.format_preference = formats.into();
    self
  }

  /// Preference-ordered list of presentation modes. Defaults to
  /// [`vk::PresentModeKHR::FIFO_KHR`].
  #[inline]
  pub fn present_mode_preference(mut self, modes: &[vk::PresentModeKHR]) -> Self {
    self.present_mode_preference = modes.into();
    self
  }

  /// Required swapchain image usage flags. Defaults to
  /// [`vk::ImageUsageFlags::COLOR_ATTACHMENT`].
  #[inline]
  pub fn usage(mut self, usage: vk::ImageUsageFlags) -> Self {
    self.usage = usage;
    self
  }

  /// Requires swapchain image sharing mode. Defaults to
  /// [`vk::SharingMode::EXCLUSIVE`].
  #[inline]
  pub fn sharing_mode(mut self, mode: vk::SharingMode) -> Self {
    self.sharing_mode = mode;
    self
  }

  /// Requires swapchain image composite alpha. Defaults to
  /// [`vk::CompositeAlphaFlagsKHR::OPAQUE_KHR`].
  #[inline]
  pub fn composite_alpha(mut self, value: vk::CompositeAlphaFlagsKHR) -> Self {
    self.composite_alpha = value;
    self
  }
}

impl Default for SwapchainOptions {
  fn default() -> Self {
    Self {
      frames_in_flight: 2,
      format_preference: vec![
        vk::SurfaceFormatKHR {
          format: vk::Format::B8G8R8A8_SRGB,
          color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        },
        vk::SurfaceFormatKHR {
          format: vk::Format::R8G8B8A8_SRGB,
          color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        },
      ],
      present_mode_preference: vec![vk::PresentModeKHR::FIFO],
      usage: vk::ImageUsageFlags::COLOR_ATTACHMENT,
      sharing_mode: vk::SharingMode::EXCLUSIVE,
      composite_alpha: vk::CompositeAlphaFlagsKHR::OPAQUE,
    }
  }
}

struct Frame {
  complete: vk::Fence,
  acquire: vk::Semaphore,
  generation: u64,
}

/// Information necessary to render a frame, from [`Swapchain::acquire`]
#[derive(Debug, Copy, Clone)]
pub struct AcquiredFrame {
  /// Index of the image to write to in [`Swapchain::images`].
  pub image_index: usize,
  /// Index of the frame in flight, for use tracking your own per-frame
  /// resources, which may be accessed immediately after
  /// [`Swapchain::acquire`] returns.
  pub frame_index: usize,
  /// Must be waited on before accessing the image associated with
  /// `image_index`.
  pub ready: vk::Semaphore,
  /// Must be signaled when access to the image associated with `image_index`
  /// and any per-frame resources associated with `frame_index` is complete.
  pub complete: vk::Fence,
  /// Set whenever [`Swapchain::images`] has, and [`Swapchain::extent`] and
  /// [`Swapchain::format`] may have, changed since the last
  /// [`Swapchain::acquire`] call. Use this to invalidate derived
  /// resources like [`vk::ImageView`]s and [`vk::Framebuffer`]s, taking care
  /// not to destroy them until at least
  /// [`SwapchainOptions::frames_in_flight`] new frames have been acquired,
  /// including this one.
  pub invalidate_images: bool,
}
