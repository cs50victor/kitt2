#![allow(unused_parens, non_snake_case)]
use anyhow::{Context, Result};
use log::{info, warn};
use std::sync::Arc;
use vulkano::{
    device::{
        physical::{PhysicalDevice, PhysicalDeviceType},
        Device, DeviceCreateInfo, DeviceExtensions, Features, Properties, Queue, QueueCreateInfo,
        QueueFlags,
    },
    instance::{Instance, InstanceCreateInfo},
    Version, VulkanLibrary,
};

pub struct Engine {
    vkdevice: Arc<Device>,
    gfx_queue: Arc<Queue>,
    instance: Arc<Instance>,
    gfx_queue_family_index: u32,
    num_of_triangles: u64,
    num_of_vertices: u64,
    avg_fps: f32,
}

impl Engine {
    pub fn new() -> Result<Self> {
        warn!("ONLY WORKS / TESTED WITH VULKAN V1.3.250.1");
        let library = VulkanLibrary::new().context("no local Vulkan library/DLL")?;
        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                #[cfg(target_os = "macos")]
                enumerate_portability: true,
                ..Default::default()
            },
        )
        .context("failed to create instance")?;

        let mut device_extensions = DeviceExtensions::empty();

        let features = Features {
            dynamic_rendering: true,
            fill_mode_non_solid: true,
            multi_draw_indirect: true,
            runtime_descriptor_array: true,
            descriptor_binding_partially_bound: true,
            descriptor_binding_variable_descriptor_count: true,
            shader_sampled_image_array_non_uniform_indexing: true,
            // extended_dynamic_state: true,
            ..Features::empty()
        };

        let (processing_device, queue_family_index) =
            Self::select_processing_device(&instance, &device_extensions, &features);

        if processing_device.api_version() < Version::V1_3 {
            device_extensions.khr_dynamic_rendering = true;
        }

        // a device represents an open channel of communication with the GPU/CPU.
        let (vkdevice, mut queues) = Device::new(
            processing_device,
            DeviceCreateInfo {
                enabled_extensions: device_extensions,
                enabled_features: features,
                queue_create_infos: vec![QueueCreateInfo {
                    queue_family_index,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .context("failed to create device")?;

        // queues are used to submit work to the device. They are created along with the device.
        // they are somewhat like the GPU equivalent of CPU threads.
        let gfx_queue = queues.next().expect("failed to get first queue in iterator");

        Ok(Self {
            vkdevice,
            gfx_queue,
            gfx_queue_family_index: queue_family_index,
            instance,
            avg_fps: 0.0,
            num_of_triangles: 0,
            num_of_vertices: 0,
        })
    }

    pub fn get_vkdevice(&self) -> Arc<Device> {
        self.vkdevice.clone()
    }

    pub fn get_vkdevice_properties(&self) -> vulkano::device::Properties {
        self.vkdevice.physical_device().properties().clone()
    }

    pub fn get_gfx_queue(&self) -> Arc<Queue> {
        self.gfx_queue.clone()
    }

    pub fn get_instance(&self) -> Arc<Instance> {
        self.instance.clone()
    }

    pub fn get_avg_fps(&self) -> f32 {
        self.avg_fps
    }

    pub fn num_of_triangles(&self) -> u64 {
        self.num_of_triangles
    }

    pub fn num_of_vertices(&self) -> u64 {
        self.num_of_vertices
    }

    pub fn gfx_queue_family_index(&self) -> u32 {
        self.gfx_queue_family_index
    }

    fn select_processing_device(
        instance: &Arc<Instance>,
        device_extensions: &DeviceExtensions,
        features: &Features,
    ) -> (Arc<PhysicalDevice>, u32) {
        // processing device (CPU/GPU) to connect to
        info!("Available devices:");
        let (processing_device, queue_family_index) = instance
            .enumerate_physical_devices()
            .expect("could not enumerate devices")
            .filter(|p| {
                let Properties { device_name, device_type, .. } = &p.properties();
                info!("- {} | {:?} | Vulkan v{:?}", device_name, device_type, p.api_version());
                p.api_version() >= Version::V1_3 || p.supported_extensions().khr_dynamic_rendering
            })
            .filter(|p| p.supported_extensions().contains(device_extensions))
            .filter(|p| p.supported_features().contains(features))
            .filter_map(|p| {
                p.queue_family_properties()
                    .iter()
                    .enumerate()
                    .position(|(_, q)| {
                        q.queue_flags.intersects(QueueFlags::GRAPHICS)
                        // && p.surface_support(i as u32, &surface).unwrap_or(false)
                    })
                    .map(|q| (p, q as u32))
            })
            .min_by_key(|(p, _)| match p.properties().device_type {
                PhysicalDeviceType::DiscreteGpu => 0,
                PhysicalDeviceType::IntegratedGpu => 1,
                PhysicalDeviceType::VirtualGpu => 2,
                PhysicalDeviceType::Cpu => 3,
                PhysicalDeviceType::Other => 4,
                _ => 5,
            })
            .expect("no devices available");

        info!("* Using {}", processing_device.properties().device_name,);

        (processing_device, queue_family_index)
    }
}
