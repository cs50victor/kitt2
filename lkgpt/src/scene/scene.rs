use anyhow::{bail, Result};
use log::info;
use std::{
    fs::File,
    io::{Read, Write},
    sync::Arc,
};

use crate::assets::{gltf::load_external_gltf, mesh::MeshVertex, model::Model, texture::Texture};
use glam::Mat4;

use vulkano::{
    buffer::{
        allocator::{SubbufferAllocator, SubbufferAllocatorCreateInfo},
        subbuffer::BufferReadGuard,
        Buffer, BufferCreateInfo, BufferUsage, Subbuffer,
    },
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
        CopyImageToBufferInfo, PrimaryAutoCommandBuffer, RenderPassBeginInfo, SubpassContents,
    },
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator,
        layout::{
            DescriptorSetLayout, DescriptorSetLayoutBinding, DescriptorSetLayoutCreateInfo,
            DescriptorSetLayoutCreationError, DescriptorType,
        },
        PersistentDescriptorSet, WriteDescriptorSet,
    },
    device::{Device, DeviceOwned},
    format::Format,
    image::{view::ImageView, AttachmentImage, ImageAccess, ImageDimensions, StorageImage},
    memory::allocator::{AllocationCreateInfo, MemoryUsage, StandardMemoryAllocator},
    pipeline::{
        cache::PipelineCache,
        graphics::{
            depth_stencil::DepthStencilState,
            input_assembly::InputAssemblyState,
            vertex_input::Vertex,
            viewport::{Viewport, ViewportState},
        },
        layout::{PipelineLayoutCreateInfo, PushConstantRange},
        GraphicsPipeline, Pipeline, PipelineBindPoint, PipelineLayout,
    },
    render_pass::{Framebuffer, FramebufferCreateInfo, Subpass},
    sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo},
    shader::{ShaderModule, ShaderStages},
    single_pass_renderpass,
};

use crate::scene::{
    material_manager::OrderedMaterialsMap, node::Node, texture_manager::OrderedTexturesMap,
};

pub struct Scene {
    root: Node,
    vertex_shader: Arc<ShaderModule>,
    fragment_shader: Arc<ShaderModule>,
    descriptor_set_allocator: StandardDescriptorSetAllocator,
    texture_sampler: Arc<Sampler>,
    // remove pub later
    pub all_materials: OrderedMaterialsMap,
    memory_allocator: Arc<StandardMemoryAllocator>,
    cmd_buffer_builder:
        AutoCommandBufferBuilder<PrimaryAutoCommandBuffer, Arc<StandardCommandBufferAllocator>>,
    camera_subbuffer_allocator: SubbufferAllocator,
    camera_subbuffer: Option<Subbuffer<vs::UniformBufferObject>>,
    pipeline_cache: Arc<PipelineCache>,
    pipeline_layout: Arc<PipelineLayout>,
    // remove pub later
    pub all_textures: OrderedTexturesMap,
    cmd_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue_family_index: u32,
    scene_img: Arc<StorageImage>,
    scene_img_buffer: Subbuffer<[u8]>,
}

impl Scene {
    const CAMERA_BINDING: u32 = 0;
    // tiktok, facetime, (vertical video) [1080, 1920]
    const DEFAULT_RESOLUTION: [u32; 2] = [1920, 1080];
    const MAIN_SET: usize = 0;
    const MATERIALS_BINDING: u32 = 1;
    // gotten from error message - MaxPerStageDescriptorSamplersExceeded
    pub const MAX_DESCRIPTOR_COUNT: u32 = 15;
    const SCENE_ROOT_ID: u32 = 0;
    const TEXTURES_BINDING: u32 = 0;
    const TEXTURE_SET: usize = 1;

    pub fn new(device: Arc<Device>, queue_family_index: u32) -> Result<Self> {
        let texture_sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::Repeat; 3],
                ..Default::default()
            },
        )?;

        let descriptor_set_allocator = StandardDescriptorSetAllocator::new(device.clone());

        let root = Node::new(Self::SCENE_ROOT_ID, None, None, None);

        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));

        let cmd_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));

        let vs = vs::load(device.clone()).expect("failed to load vertex shader");
        let fs = fs::load(device.clone()).expect("failed to load fragment shader");

        let camera_subbuffer_allocator = SubbufferAllocator::new(
            memory_allocator.clone(),
            SubbufferAllocatorCreateInfo {
                buffer_usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
        );

        let pipeline_layout = Self::create_pipeline_layout(device.clone())?;

        let pipeline_cache = Self::retrieve_or_create_pipeline_cache(device)?;

        let cmd_buffer_builder =
            Self::create_cmd_buffer_builder(cmd_buffer_allocator.clone(), queue_family_index)?;

        let scene_img = Self::create_scene_img(
            Self::DEFAULT_RESOLUTION,
            queue_family_index,
            &memory_allocator,
        )?;

        let scene_img_buffer = Buffer::from_iter(
            &memory_allocator,
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                usage: MemoryUsage::Upload,
                ..Default::default()
            },
            (0..Self::DEFAULT_RESOLUTION[0] * Self::DEFAULT_RESOLUTION[1] * 4).map(|_| 0u8),
        )?;

        Ok(Self {
            root,
            camera_subbuffer_allocator,
            camera_subbuffer: None,
            memory_allocator,
            descriptor_set_allocator,
            texture_sampler,
            cmd_buffer_builder,
            vertex_shader: vs,
            fragment_shader: fs,
            pipeline_layout,
            pipeline_cache,
            all_materials: OrderedMaterialsMap::new(),
            all_textures: OrderedTexturesMap::new(),
            cmd_buffer_allocator,
            queue_family_index,
            scene_img,
            scene_img_buffer,
        })
    }

    pub fn create_storage_imgs(&self) -> Vec<Arc<StorageImage>> {
        let [width, height] = self.width_height();
        (0..1)
            .map(|_| {
                StorageImage::new(
                    &self.memory_allocator,
                    ImageDimensions::Dim2d {
                        width,
                        height,
                        array_layers: 1,
                    },
                    Format::R8G8B8A8_UNORM,
                    Some(self.queue_family_index),
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    }

    pub fn add_node(&mut self, node: Node) {
        self.root.add_node(node);
    }

    pub fn create_node(&mut self, model: Option<Model>) -> Node {
        Node::new(self.num_of_nodes() + 1, Some(self), None, model)
    }

    pub fn load_gltf(&mut self, gltf_path: &str) -> Result<Model> {
        load_external_gltf(
            &self.memory_allocator,
            &mut self.cmd_buffer_builder,
            gltf_path,
        )
    }

    pub fn prepare_and_bind_to_cmd_buffer(
        &mut self,
        pipeline: &Arc<GraphicsPipeline>,
        camera_model_matrix: Mat4,
    ) -> Result<()> {
        let cam_buffer = match self.camera_subbuffer.take() {
            Some(cam_buffer) => cam_buffer,
            None => {
                bail!("camera subbuffer shouldn't be None - did you forget to call update_camera?")
            }
        };

        let set_zero_layout = match pipeline.layout().set_layouts().get(Self::MAIN_SET){
            Some(set_zero_layout) => set_zero_layout,
            None => bail!("pipeline layout doesn't have a set 0 (MAIN_SET). Check the pipeline layout creation and your shader code"),
        };

        let set_one_layout = match pipeline.layout().set_layouts().get(Self::TEXTURE_SET){
            Some(set_one_layout) => set_one_layout,
            None => bail!("pipeline layout doesn't have a set 1 (TEXTURE_SET). Check the pipeline layout creation and your shader code"),
        };

        let materials_buffer = Buffer::from_iter(
            &self.memory_allocator,
            BufferCreateInfo {
                usage: BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                usage: MemoryUsage::Upload,
                ..Default::default()
            },
            self.all_materials
                .iter()
                .map(|(_, mat)| mat.to_gpu_material())
                .collect::<Vec<_>>(),
        )?;

        let camera_and_material_set = PersistentDescriptorSet::new(
            self.descriptor_set_allocator(),
            set_zero_layout.clone(),
            [
                WriteDescriptorSet::buffer(Self::CAMERA_BINDING, cam_buffer),
                WriteDescriptorSet::buffer(Self::MATERIALS_BINDING, materials_buffer),
            ],
        )?;

        let texture_set = PersistentDescriptorSet::new_variable(
            self.descriptor_set_allocator(),
            set_one_layout.clone(),
            self.all_textures.len() as u32,
            [WriteDescriptorSet::image_view_sampler_array(
                Self::TEXTURES_BINDING,
                0,
                self.all_textures
                    .iter()
                    .map(|(_, tex)| (tex.image_view.clone() as _, self.texture_sampler.clone())),
            )],
        )?;

        self.cmd_buffer_builder.bind_descriptor_sets(
            PipelineBindPoint::Graphics,
            pipeline.layout().clone(),
            0,
            vec![camera_and_material_set, texture_set],
        );

        let mut vertex_buffer_arr: Vec<Subbuffer<[MeshVertex]>> = vec![];
        let mut indicies_arr: Vec<u32> = vec![];

        self.root.get_index_and_vertex_buffers(
            &mut vertex_buffer_arr,
            &mut indicies_arr,
            &mut self.cmd_buffer_builder,
        );

        self.cmd_buffer_builder
            .bind_vertex_buffers(0, vertex_buffer_arr);

        // self.cmd_buffer_builder.bind_index_buffer(Buffer::from_iter(
        //     &self.memory_allocator,
        //     BufferCreateInfo { usage: BufferUsage::INDEX_BUFFER, ..Default::default() },
        //     AllocationCreateInfo { usage: MemoryUsage::Upload, ..Default::default() },
        //     indicies_arr,
        // )?);

        self.cmd_buffer_builder.push_constants(
            pipeline.layout().clone(),
            0,
            vs::PushConstants {
                model: camera_model_matrix.to_cols_array_2d(),
            },
        );

        self.cmd_buffer_builder
            .draw_indexed(self.root.num_of_indices_in_tree() as u32, 1, 0, 0, 0)?
            .end_render_pass()?;
        Ok(())
    }

    pub fn num_of_nodes(&self) -> u32 {
        self.root.get_num_of_descendants()
    }

    pub fn num_of_vertices(&self) -> u64 {
        self.root.num_of_vertices_in_tree()
    }

    pub fn num_of_indices(&self) -> u64 {
        self.root.num_of_indices_in_tree()
    }

    pub fn num_of_textures(&self) -> u8 {
        self.all_textures.len()
    }

    pub fn num_of_materials(&self) -> u8 {
        self.all_materials.len()
    }

    pub fn add_texture(&mut self, texture: Texture) -> u8 {
        let index = self.all_textures.len();
        self.all_textures.insert(index, texture);
        index + 1
    }

    pub fn descriptor_set_allocator(&self) -> &StandardDescriptorSetAllocator {
        &self.descriptor_set_allocator
    }

    pub fn memory_allocator(&self) -> Arc<StandardMemoryAllocator> {
        self.memory_allocator.clone()
    }

    pub fn width_height(&self) -> [u32; 2] {
        self.scene_img.dimensions().width_height()
    }

    pub fn new_gfx_pipeline_and_frame_buffer(
        &self,
    ) -> Result<(Arc<GraphicsPipeline>, Arc<Framebuffer>)> {
        // This function is called once during initialization, then again whenever the window is resized.

        let dimensions = self.width_height();

        let depth_buffer = ImageView::new_default(AttachmentImage::transient(
            &self.memory_allocator,
            dimensions,
            Format::D16_UNORM,
        )?)?;

        let render_pass = single_pass_renderpass!(
            self.memory_allocator.device().clone(),
            attachments: {
                color: {
                    load: Clear,
                    store: Store,
                    format: self.scene_img.format(),
                    samples: 1,
                },
                depth: {
                    load: Clear,
                    store: DontCare,
                    format: Format::D16_UNORM,
                    samples: 1,
                },
            },
            pass: {
                color: [color],
                depth_stencil: {depth},
            },
        )?;

        let frame_buffer = {
            let image_view = ImageView::new_default(self.scene_img.clone())?;
            Framebuffer::new(
                render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![image_view, depth_buffer],
                    ..Default::default()
                },
            )?
        };

        let pipeline = GraphicsPipeline::start()
            .build_with_cache(self.pipeline_cache.clone())
            .vertex_input_state(MeshVertex::per_vertex())
            .vertex_shader(
                self.vertex_shader
                    .entry_point("main")
                    .expect("invalid vertex shader entry point"),
                (),
            )
            .input_assembly_state(InputAssemblyState::new())
            .viewport_state(ViewportState::viewport_fixed_scissor_irrelevant([
                Viewport {
                    origin: [0.0, 0.0],
                    dimensions: [dimensions[0] as f32, dimensions[1] as f32],
                    depth_range: 0.0..1.0,
                },
            ]))
            .fragment_shader(
                self.fragment_shader
                    .entry_point("main")
                    .expect("invalid fragment shader entry point"),
                (),
            )
            .depth_stencil_state(DepthStencilState::simple_depth_test())
            .render_pass(Subpass::from(render_pass, 0).unwrap())
            .with_pipeline_layout(
                self.memory_allocator.device().clone(),
                self.pipeline_layout.clone(),
            )?;

        info!("Graphics Pipleline Created");
        self.save_pipeline_cache()?;

        // build_with_cache
        Ok((pipeline, frame_buffer))
    }

    pub fn build_primary_cmd_buffer(
        &mut self,
        pipeline: &Arc<GraphicsPipeline>,
        framebuffer: Arc<Framebuffer>,
        camera_model_matrix: Mat4,
    ) -> Result<PrimaryAutoCommandBuffer> {
        self.cmd_buffer_builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into()), Some(1f32.into())],
                    ..RenderPassBeginInfo::framebuffer(framebuffer)
                },
                SubpassContents::Inline,
            )?
            .bind_pipeline_graphics(pipeline.clone());

        self.prepare_and_bind_to_cmd_buffer(pipeline, camera_model_matrix)?;

        self.cmd_buffer_builder
            .copy_image_to_buffer(CopyImageToBufferInfo::image_buffer(
                self.scene_img.clone(),
                self.scene_img_buffer.clone(),
            ))?;

        let new_cmd_buffer_builder = Self::create_cmd_buffer_builder(
            self.cmd_buffer_allocator.clone(),
            self.queue_family_index,
        )?;
        let prev_cmd_buffer_builder =
            std::mem::replace(&mut self.cmd_buffer_builder, new_cmd_buffer_builder);

        Ok(prev_cmd_buffer_builder.build()?)
    }

    pub fn img_buffer_content(&self) -> Result<BufferReadGuard<[u8]>> {
        Ok(self.scene_img_buffer.read()?)
    }

    pub fn update_camera_subbuffer_allocator(&mut self, camera_info: (Mat4, Mat4)) -> Result<()> {
        let uniform_data = vs::UniformBufferObject {
            view: camera_info.0.to_cols_array_2d(),
            proj: camera_info.1.to_cols_array_2d(),
        };

        let subbuffer = self.camera_subbuffer_allocator.allocate_sized()?;
        *subbuffer.write()? = uniform_data;

        self.camera_subbuffer = Some(subbuffer);
        Ok(())
    }

    pub fn print_stats(&self) {
        info!(
            "num of nodes: {} | num of vertices: {} | num of indices: {}",
            self.num_of_nodes(),
            self.num_of_vertices(),
            self.num_of_indices()
        );
    }

    pub fn create_pipeline_layout(device: Arc<Device>) -> Result<Arc<PipelineLayout>> {
        // CAMERA, MATERIALS, TEXTURES . in order
        let layout_create_infos = vec![
            DescriptorSetLayoutCreateInfo {
                bindings: [
                    (
                        Self::CAMERA_BINDING,
                        DescriptorSetLayoutBinding {
                            stages: ShaderStages::VERTEX,
                            ..DescriptorSetLayoutBinding::descriptor_type(
                                DescriptorType::UniformBuffer,
                            )
                        },
                    ),
                    (
                        Self::MATERIALS_BINDING,
                        DescriptorSetLayoutBinding {
                            stages: ShaderStages::FRAGMENT,
                            ..DescriptorSetLayoutBinding::descriptor_type(
                                DescriptorType::StorageBuffer,
                            )
                        },
                    ),
                ]
                .into(),
                ..Default::default()
            },
            DescriptorSetLayoutCreateInfo {
                bindings: [(
                    Self::TEXTURES_BINDING,
                    DescriptorSetLayoutBinding {
                        stages: ShaderStages::FRAGMENT,
                        variable_descriptor_count: true,
                        descriptor_count: Self::MAX_DESCRIPTOR_COUNT,
                        ..DescriptorSetLayoutBinding::descriptor_type(
                            DescriptorType::CombinedImageSampler,
                        )
                    },
                )]
                .into(),
                ..Default::default()
            },
        ];

        let set_layouts = layout_create_infos
            .into_iter()
            .map(|desc| DescriptorSetLayout::new(device.clone(), desc))
            .collect::<Result<Vec<_>, DescriptorSetLayoutCreationError>>()?;

        let pipeline_layout = PipelineLayout::new(
            device,
            PipelineLayoutCreateInfo {
                set_layouts,
                push_constant_ranges: vec![PushConstantRange {
                    stages: ShaderStages::VERTEX,
                    offset: 0,
                    size: std::mem::size_of::<Mat4>() as u32,
                }],
                ..Default::default()
            },
        )?;

        Ok(pipeline_layout)
    }

    fn create_cmd_buffer_builder(
        cmd_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        queue_family_index: u32,
    ) -> Result<
        AutoCommandBufferBuilder<PrimaryAutoCommandBuffer, Arc<StandardCommandBufferAllocator>>,
    > {
        Ok(AutoCommandBufferBuilder::primary(
            &cmd_buffer_allocator,
            queue_family_index,
            CommandBufferUsage::OneTimeSubmit,
        )?)
    }

    pub fn scene_img(&self) -> &StorageImage {
        self.scene_img.as_ref()
    }

    fn retrieve_or_create_pipeline_cache(device: Arc<Device>) -> Result<Arc<PipelineCache>> {
        let data = {
            match File::open("pipeline_cache.bin") {
                Ok(mut file) => {
                    let mut data = Vec::new();
                    match file.read_to_end(&mut data) {
                        Ok(_) => {
                            info!("Using pipeline cache from file");
                            Some(data)
                        }
                        Err(_) => {
                            info!("Failed to read pipeline cache file");
                            None
                        }
                    }
                }
                Err(_) => None,
            }
        };
        let pipeline_cache = match data {
            // This is unsafe because there is no way to be sure that the file contains valid data.
            Some(data) => unsafe { PipelineCache::with_data(device, &data)? },
            None => PipelineCache::empty(device)?,
        };

        Ok(pipeline_cache)
    }

    fn save_pipeline_cache(&self) -> Result<()> {
        match self.pipeline_cache.get_data() {
            Ok(data) => {
                // TODO - try to avoid writing to disk if the data is the same
                // (i.e. if the cache is already up to date)
                // why not open and overwrite the file directly?
                if let Ok(mut file) = File::create("pipeline_cache.bin.tmp") {
                    if file.write_all(&data).is_ok() {
                        std::fs::rename("pipeline_cache.bin.tmp", "pipeline_cache.bin")?;
                    } else {
                        std::fs::remove_file("pipeline_cache.bin.tmp")?;
                    }
                }
            }
            Err(err) => {
                bail!("Error while getting pipeline cache data: {:?}", err);
            }
        };
        Ok(())
    }

    fn create_scene_img(
        dimensions: [u32; 2],
        queue_family_index: u32,
        memory_allocator: &Arc<StandardMemoryAllocator>,
    ) -> Result<Arc<StorageImage>> {
        let [width, height] = dimensions;
        if height % 2 != 0 || width % 2 != 0 {
            bail!("Scene width and height must be divisible by 2");
        }

        Ok(StorageImage::new(
            memory_allocator,
            ImageDimensions::Dim2d {
                width,
                height,
                array_layers: 1,
            },
            Format::R8G8B8A8_UNORM,
            Some(queue_family_index),
        )?)
    }

    pub fn handle_input(&mut self, input: &str) {
        info!("input: {}", input);
    }
}

mod vs {
    vulkano_shaders::shader! {ty: "vertex",path: "shaders/vert.glsl"}
}
mod fs {
    vulkano_shaders::shader! {ty: "fragment", path: "shaders/frag.glsl"}
}
