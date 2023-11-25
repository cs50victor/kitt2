#![feature(ascii_char, async_closure)]
use actix_web::{middleware, web::Data, App, HttpServer};
use log::info;
use std::env;

mod assets{
    pub mod gltf{
        use anyhow::{bail, Context, Result};
        use glam::Mat4;
        use gltf::{buffer, Gltf};
        use std::{
            fs,
            path::{self, Path},
            sync::Arc,
        };
        use vulkano::{
            command_buffer::{
                allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder,
                PrimaryAutoCommandBuffer,
            },
            format,
            image::{view::ImageView, ImageDimensions, ImmutableImage, MipmapsCount},
            memory::allocator::StandardMemoryAllocator,
        };

        use super::{
            materials,
            mesh::{Mesh, MeshVertex},
            model::Model,
            texture::Texture,
        };

        pub fn load_external_gltf(
            memory_allocator: &StandardMemoryAllocator,
            cmd_buffer_builder: &mut AutoCommandBufferBuilder<
                PrimaryAutoCommandBuffer,
                Arc<StandardCommandBufferAllocator>,
            >,
            gltf_path: &str,
        ) -> Result<Model> {
            if !gltf_path.ends_with(".gltf") {
                bail!("gltf file's extension isn't .gltf");
            }

            let base_path = path::Path::new("assets");
            let full_path = base_path.join(gltf_path);
            let gltf = Gltf::open(&full_path)?;

            // Load buffers
            let buffer_data = gltf
                .buffers()
                .map(|buffer| {
                    let mut data = match buffer.source() {
                        buffer::Source::Bin => gltf
                            .blob
                            .as_deref()
                            .context("Failed to open gltf blob")?
                            .to_vec(),
                        buffer::Source::Uri(uri) => {
                            let uri = full_path
                                .parent()
                                .unwrap_or_else(|| Path::new("./"))
                                .join(uri);
                            let uri_display = uri.display().to_string();
                            fs::read(uri)
                                .with_context(|| format!("Failed to read buffer uri: {}", uri_display))?
                        }
                    };
                    if data.len() < buffer.length() {
                        bail!(
                            "Buffer length is too short, expected {}, got {}",
                            buffer.length(),
                            data.len()
                        );
                    }
                    while data.len() % 4 != 0 {
                        data.push(0);
                    }
                    Ok(data)
                })
                .collect::<Result<Vec<Vec<u8>>>>()?;

            let textures = gltf
                .images()
                .map(|img| match img.source() {
                    gltf::image::Source::Uri { uri, .. } => {
                        if !(uri.ends_with(".png") || uri.ends_with(".jpeg") || uri.ends_with(".jpg")) {
                            bail!("only png, jpeg, and jpg texture images are supported {uri}")
                        }
                        let uri = full_path
                            .parent()
                            .unwrap_or_else(|| Path::new("./"))
                            .join(uri)
                            .display()
                            .to_string();
                        let data = fs::read(&uri)
                            .with_context(|| format!("Failed to read image uri: {}", &uri))?;

                        let image = image::load_from_memory(&data)
                            .with_context(|| format!("Failed to decode image bytes: {}", uri))?
                            // .grayscale()
                            .to_rgba8();

                        let (width, height) = image.dimensions();
                        let dimensions = ImageDimensions::Dim2d {
                            width,
                            height,
                            array_layers: 1,
                        };

                        let image_data = image.into_raw();
                        let image = ImmutableImage::from_iter(
                            memory_allocator,
                            image_data,
                            dimensions,
                            MipmapsCount::Log2,
                            format::Format::R8G8B8A8_UNORM,
                            cmd_buffer_builder,
                        )?;
                        let image_view = ImageView::new_default(image)?;

                        Ok(Texture { image_view })
                    }
                    _ => bail!("Only external images textures are supported"),
                })
                .collect::<Result<Vec<_>>>()?;

            let materials = gltf
                .materials()
                .map(materials::get_material_data)
                .collect::<Vec<_>>();

            let meshes = gltf
                .meshes()
                .flat_map(|mesh| {
                    mesh.primitives()
                        .map(|mesh_primitive| {
                            let material_index =
                                mesh_primitive.material().index().unwrap_or_default() as u8;

                            let reader = mesh_primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

                            let normals = reader
                                .read_normals()
                                .map_or(Vec::new(), |normals| normals.collect());

                            let colors = reader
                                .read_colors(0)
                                .map_or(Vec::new(), |colors| colors.into_rgba_f32().collect());

                            let tangents = reader
                                .read_tangents()
                                .map_or(Vec::new(), |tangents| tangents.collect());

                            let tex_coords = reader
                                .read_tex_coords(0)
                                .map_or(Vec::new(), |tex_coords| tex_coords.into_f32().collect());

                            let vertices = reader.read_positions().map_or(Vec::new(), |positions| {
                                let normals_len = normals.len();
                                let tex_coords_len = tex_coords.len();
                                let colors_len = colors.len();
                                let tangents_len = tangents.len();
                                positions
                                    .enumerate()
                                    .map(|(i, position)| {
                                        let normal = if i >= normals_len {
                                            Default::default()
                                        } else {
                                            normals[i]
                                        };
                                        let tex_coords = if i >= tex_coords_len {
                                            Default::default()
                                        } else {
                                            tex_coords[i]
                                        };
                                        let color = if i >= colors_len {
                                            Default::default()
                                        } else {
                                            colors[i]
                                        };
                                        let tangent = if i >= tangents_len {
                                            Default::default()
                                        } else {
                                            tangents[i]
                                        };

                                        MeshVertex {
                                            position,
                                            normal,
                                            tex_coords,
                                            color,
                                            tangent,
                                            material_index,
                                        }
                                    })
                                    .collect()
                            });

                            let indices = reader
                                .read_indices()
                                .map_or(Vec::new(), |i| i.into_u32().collect());

                            Mesh::new(memory_allocator, vertices, indices, material_index).with_context(
                                || {
                                    format!(
                                        "Failed to load gltf mesh: {}",
                                        mesh.name().unwrap_or("Unnamed")
                                    )
                                },
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(Model {
                meshes,
                transforms: vec![Mat4::IDENTITY],
                textures,
                materials,
            })
        }

    }
    pub mod materials{
        use gltf::material::AlphaMode;
        use vulkano::buffer::BufferContents;

        #[derive(Clone, Debug)]
        pub struct MeshMaterial {
            name: String,
            alpha_cutoff: Option<f32>,
            alpha_mode: AlphaMode,
            base_color_factor: [f32; 4],
            base_color_texture_index: Option<u8>,
            double_sided: bool,
            emissive_factor: [f32; 3],
            emissive_texture_index: Option<u8>,
            ior: Option<f32>,
            metallic_roughness_texture_index: Option<u8>,
            normal_texture_index: Option<u8>,
            occlusion_texture_index: Option<u8>,
            specular_color_texture_index: Option<u8>,
            specular_texture_index: Option<u8>,
            specular_diffuse_texture_index: Option<u8>,
            specular_glossiness_texture_index: Option<u8>,
            transmission_texture_index: Option<u8>,
            unlit: bool,
            vol_thickness_texture_index: Option<u8>,
            vol_thickness_factor: Option<f32>,
            vol_attenuation_distance: Option<f32>,
            vol_attenuation_color: Option<[f32; 3]>,
        }

        #[derive(BufferContents, Clone, Copy, Debug)]
        #[repr(C)]
        pub struct GpuMaterial {
            base_color_factor: [f32; 4],
            base_color_texture_index: u8,
            emissive_factor: [f32; 3],
            emissive_texture_index: u8,
            ior: f32,
            metallic_roughness_texture_index: u8,
            normal_texture_index: u8,
            occlusion_texture_index: u8,
            specular_color_texture_index: u8,
            specular_texture_index: u8,
            specular_diffuse_texture_index: u8,
            specular_glossiness_texture_index: u8,
            transmission_texture_index: u8,
            unlit: u8, // 1 for true, 0 for false
            vol_thickness_texture_index: u8,
            vol_thickness_factor: f32,
            vol_attenuation_distance: f32,
            vol_attenuation_color: [f32; 3],
        }

        impl MeshMaterial {
            pub fn update_texture_indexs(&mut self, scene_tex_arr_len: u8) {
                if let Some(base_color_texture_index) = &mut self.base_color_texture_index {
                    *base_color_texture_index += scene_tex_arr_len;
                };
                if let Some(emissive_texture_index) = &mut self.emissive_texture_index {
                    *emissive_texture_index += scene_tex_arr_len;
                };

                if let Some(metallic_roughness_texture_index) = &mut self.metallic_roughness_texture_index {
                    *metallic_roughness_texture_index += scene_tex_arr_len;
                };

                if let Some(normal_texture_index) = &mut self.normal_texture_index {
                    *normal_texture_index += scene_tex_arr_len;
                };

                if let Some(occlusion_texture_index) = &mut self.occlusion_texture_index {
                    *occlusion_texture_index += scene_tex_arr_len;
                };

                if let Some(specular_color_texture_index) = &mut self.specular_color_texture_index {
                    *specular_color_texture_index += scene_tex_arr_len;
                };

                if let Some(specular_texture_index) = &mut self.specular_texture_index {
                    *specular_texture_index += scene_tex_arr_len;
                };

                if let Some(specular_diffuse_texture_index) = &mut self.specular_diffuse_texture_index {
                    *specular_diffuse_texture_index += scene_tex_arr_len;
                };

                if let Some(specular_glossiness_texture_index) = &mut self.specular_glossiness_texture_index
                {
                    *specular_glossiness_texture_index += scene_tex_arr_len;
                };

                if let Some(transmission_texture_index) = &mut self.transmission_texture_index {
                    *transmission_texture_index += scene_tex_arr_len;
                };

                if let Some(vol_thickness_texture_index) = &mut self.vol_thickness_texture_index {
                    *vol_thickness_texture_index += scene_tex_arr_len;
                };
            }

            pub fn to_gpu_material(&self) -> GpuMaterial {
                GpuMaterial {
                    base_color_factor: self.base_color_factor,
                    base_color_texture_index: self
                        .base_color_texture_index
                        .map_or(Default::default(), |value| value),
                    emissive_factor: self.emissive_factor,
                    emissive_texture_index: self
                        .emissive_texture_index
                        .map_or(Default::default(), |value| value),
                    ior: self.ior.map_or(Default::default(), |value| value),
                    metallic_roughness_texture_index: self
                        .metallic_roughness_texture_index
                        .map_or(Default::default(), |value| value),
                    normal_texture_index: self
                        .normal_texture_index
                        .map_or(Default::default(), |value| value),
                    occlusion_texture_index: self
                        .occlusion_texture_index
                        .map_or(Default::default(), |value| value),
                    specular_color_texture_index: self
                        .specular_color_texture_index
                        .map_or(Default::default(), |value| value),
                    specular_texture_index: self
                        .specular_texture_index
                        .map_or(Default::default(), |value| value),
                    specular_diffuse_texture_index: self
                        .specular_diffuse_texture_index
                        .map_or(Default::default(), |value| value),
                    specular_glossiness_texture_index: self
                        .specular_glossiness_texture_index
                        .map_or(Default::default(), |value| value),
                    transmission_texture_index: self
                        .transmission_texture_index
                        .map_or(Default::default(), |value| value),
                    unlit: self.unlit.into(),
                    vol_thickness_texture_index: self
                        .vol_thickness_texture_index
                        .map_or(Default::default(), |value| value),
                    vol_thickness_factor: self
                        .vol_thickness_factor
                        .map_or(Default::default(), |value| value),
                    vol_attenuation_distance: self
                        .vol_attenuation_distance
                        .map_or(Default::default(), |value| value),
                    vol_attenuation_color: self
                        .vol_attenuation_color
                        .map_or(Default::default(), |value| value),
                }
            }
        }

        pub fn get_material_data(material: gltf::Material) -> MeshMaterial {
            let name = material
                .name()
                .map_or("UnNamed".to_string(), |name| name.to_string());

            let pbr_metallic = material.pbr_metallic_roughness();
            // ----
            let alpha_cutoff = material.alpha_cutoff();
            let alpha_mode = material.alpha_mode();
            let base_color_factor = pbr_metallic.base_color_factor();
            let base_color_texture_index = pbr_metallic
                .base_color_texture()
                .map(|info| info.texture().index() as u8);
            let double_sided = material.double_sided();
            let emissive_factor = material.emissive_factor();
            let emissive_texture_index = material
                .emissive_texture()
                .map(|info| info.texture().index() as u8);
            let ior = material.ior();
            let metallic_roughness_texture_index = pbr_metallic
                .metallic_roughness_texture()
                .map(|info| info.texture().index() as u8);
            let normal_texture_index = material
                .normal_texture()
                .map(|normal| normal.texture().index() as u8);
            let occlusion_texture_index = material
                .occlusion_texture()
                .map(|occlusion| occlusion.texture().index() as u8);
            let (specular_color_texture_index, specular_texture_index) =
                material.specular().map_or((None, None), |specular| {
                    let specular_color_texture_index = specular
                        .specular_color_texture()
                        .map(|info| info.texture().index() as u8);
                    let specular_texture_index = specular
                        .specular_texture()
                        .map(|info| info.texture().index() as u8);
                    (specular_color_texture_index, specular_texture_index)
                });
            let (specular_diffuse_texture_index, specular_glossiness_texture_index) = material
                .pbr_specular_glossiness()
                .map_or((None, None), |pbr_gloss| {
                    let specular_diffuse_texture_index = pbr_gloss
                        .diffuse_texture()
                        .map(|info| info.texture().index() as u8);
                    let specular_glossiness_texture_index = pbr_gloss
                        .specular_glossiness_texture()
                        .map(|info| info.texture().index() as u8);
                    (
                        specular_diffuse_texture_index,
                        specular_glossiness_texture_index,
                    )
                });
            let transmission_texture_index = material.transmission().and_then(|transmission| {
                transmission
                    .transmission_texture()
                    .map(|info| info.texture().index() as u8)
            });
            let unlit = material.unlit();
            let (
                vol_thickness_texture_index,
                vol_thickness_factor,
                vol_attenuation_distance,
                vol_attenuation_color,
            ) = material.volume().map_or((None, None, None, None), |tex| {
                let vol_thickness_texture_index = tex
                    .thickness_texture()
                    .map(|tex| tex.texture().index() as u8);
                let vol_thickness_factor = tex.thickness_factor();
                let vol_attenuation_distance = tex.attenuation_distance();
                let vol_attenuation_color = tex.attenuation_color();
                (
                    vol_thickness_texture_index,
                    Some(vol_thickness_factor),
                    Some(vol_attenuation_distance),
                    Some(vol_attenuation_color),
                )
            });

            MeshMaterial {
                name,
                alpha_cutoff,
                alpha_mode,
                base_color_factor,
                base_color_texture_index,
                double_sided,
                emissive_factor,
                emissive_texture_index,
                ior,
                metallic_roughness_texture_index,
                normal_texture_index,
                occlusion_texture_index,
                specular_color_texture_index,
                specular_texture_index,
                specular_diffuse_texture_index,
                specular_glossiness_texture_index,
                transmission_texture_index,
                unlit,
                vol_thickness_texture_index,
                vol_thickness_factor,
                vol_attenuation_distance,
                vol_attenuation_color,
            }
        }

        /* base_color_texture,emissive_texture,metallic_roughness_texture,normal_texture,occlusion_texture,specular_color_texture,specular_texture,specular_diffuse_texture,specular_glossiness_texture,transmission_texture,volume_texture */

    }
    pub mod mesh{
        use anyhow::{Context, Result};
        use vulkano::{
            buffer::{Buffer, BufferContents, BufferCreateInfo, BufferUsage, Subbuffer},
            memory::allocator::{AllocationCreateInfo, MemoryUsage, StandardMemoryAllocator},
            pipeline::graphics::vertex_input::Vertex,
        };

        #[derive(BufferContents, Clone, Debug, Vertex)]
        #[repr(C)]
        pub struct MeshVertex {
            #[format(R32G32B32_SFLOAT)]
            pub position: [f32; 3],
            #[format(R32G32_SFLOAT)]
            pub tex_coords: [f32; 2],
            #[format(R32G32B32_SFLOAT)]
            pub normal: [f32; 3],
            #[format(R32G32B32A32_SFLOAT)]
            pub color: [f32; 4],
            #[format(R32G32B32A32_SFLOAT)]
            pub tangent: [f32; 4],
            #[format(R8_UINT)]
            pub material_index: u8,
        }

        #[derive(Clone, Debug)]
        pub struct Mesh {
            pub vertex_buffer: Subbuffer<[MeshVertex]>,
            pub index_buffer: Subbuffer<[u32]>,
            vertices: Vec<MeshVertex>,
            indices: Vec<u32>,
            material_index: u8,
        }

        impl Mesh {
            pub fn new(
                memory_allocator: &StandardMemoryAllocator,
                vertices: Vec<MeshVertex>,
                indices: Vec<u32>,
                material_index: u8,
            ) -> Result<Self> {
                let vertex_buffer = Buffer::from_iter(
                    memory_allocator,
                    BufferCreateInfo {
                        usage: BufferUsage::VERTEX_BUFFER,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        usage: MemoryUsage::Upload,
                        ..Default::default()
                    },
                    vertices.clone(),
                )
                .context("failed to create vertex buffer")?;

                let index_buffer = Buffer::from_iter(
                    memory_allocator,
                    BufferCreateInfo {
                        usage: BufferUsage::INDEX_BUFFER,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        usage: MemoryUsage::Upload,
                        ..Default::default()
                    },
                    indices.clone(),
                )
                .context("failed to create index buffer")?;

                Ok(Self {
                    vertex_buffer,
                    index_buffer,
                    vertices,
                    indices,
                    material_index,
                })
            }

            pub fn num_of_indices(&self) -> u64 {
                self.indices.len() as u64
            }

            pub fn num_of_vertices(&self) -> u64 {
                self.vertices.len() as u64
            }

            pub fn indices(&self) -> &[u32] {
                &self.indices
            }

            pub fn update_material_index(&mut self, scene_material_arr_len: u8) {
                self.material_index += scene_material_arr_len;

                self.vertex_buffer
                    .write()
                    .expect("failed to write to vertex buffer")
                    .iter_mut()
                    .for_each(|vertex| {
                        vertex.material_index += scene_material_arr_len;
                    });
            }
        }

    }
    pub mod model{
        use glam::Mat4;

        use super::{
            materials::{self, MeshMaterial},
            mesh::{self, Mesh},
            texture::{self, Texture},
        };

        #[derive(Clone, Debug)]
        pub struct Model {
            pub meshes: Vec<Mesh>,
            pub textures: Vec<Texture>,
            pub transforms: Vec<Mat4>,
            pub materials: Vec<MeshMaterial>,
        }

    }
    pub mod texture{
        use std::sync::Arc;

        use vulkano::image::{view::ImageView, ImmutableImage};

        #[derive(Clone, Debug)]
        pub struct Texture {
            pub image_view: Arc<ImageView<ImmutableImage>>,
        }

    }
}

mod scene{
    pub mod camera{

        use glam::{Mat4, Vec3};

        pub struct Camera {
            position: Vec3,
            rotation: f32,
            fov: f32,
            aspect_ratio: f32,
            scale: f32,
            view: Mat4,
            z_near: f32,
            z_far: f32,
        }

        impl Default for Camera {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Camera {
            pub fn new() -> Self {
                Self {
                    position: Vec3::new(0.0, 0.0, 0.0),
                    rotation: 0.0,
                    fov: 120.0,
                    aspect_ratio: 16.0 / 9.0,
                    z_near: 0.1,
                    z_far: 100.0,
                    scale: 0.02,
                    view: Mat4::look_at_rh(
                        Vec3::new(0.3, 0.3, 1.0),
                        Vec3::new(0.0, 0.0, 0.0),
                        Vec3::new(0.0, -1.0, 0.0),
                    ),
                }
            }

            pub fn update_aspect_ratio_from_scene(mut self, scene_dims: [u32; 2]) -> Self {
                self.aspect_ratio = scene_dims[0] as f32 / scene_dims[1] as f32;
                self
            }

            pub fn update_rotation(&mut self, rotation: f32) {
                self.rotation = rotation;
            }

            pub fn format_to_subbuffer_data(&self) -> (Mat4, Mat4) {
                let scale_matrix = Mat4::from_scale(Vec3::from_array([self.scale; 3]));

                let view_scale_dot_product = self.view * scale_matrix;

                let projection_matrix =
                    Mat4::perspective_rh(self.fov, self.aspect_ratio, self.z_near, self.z_far);

                (view_scale_dot_product, projection_matrix)
            }

            pub fn get_model_matrix(&self) -> Mat4 {
                Mat4::from_rotation_y(self.rotation)
            }
        }

    }

    mod material_manager{
        use crate::assets::materials::MeshMaterial;
        use std::collections::HashMap;

        #[derive(Debug)]
        pub struct OrderedMaterialsMap {
            map: HashMap<u8, MeshMaterial>,
            keys: Vec<u8>,
        }

        impl Default for OrderedMaterialsMap {
            fn default() -> Self {
                Self::new()
            }
        }

        impl OrderedMaterialsMap {
            pub fn new() -> Self {
                OrderedMaterialsMap {
                    map: HashMap::new(),
                    keys: Vec::new(),
                }
            }

            pub fn insert(&mut self, key: u8, value: MeshMaterial) {
                let result = self.map.insert(key, value);
                if result.is_none() {
                    self.keys.push(key);
                }
            }

            pub fn get(&self, key: &u8) -> Option<&MeshMaterial> {
                self.map.get(key)
            }

            pub fn iter(&self) -> impl Iterator<Item = (&u8, &MeshMaterial)> {
                self.keys
                    .iter()
                    .filter_map(move |k| self.map.get_key_value(k))
            }

            pub fn len(&self) -> u8 {
                self.map.len() as u8
            }
        }

    }

    mod node{
        use std::sync::Arc;

        use crate::assets::{mesh::MeshVertex, model::Model};
        use vulkano::{
            buffer::Subbuffer,
            command_buffer::{
                allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder,
                PrimaryAutoCommandBuffer,
            },
        };

        use super::scene::Scene;

        pub struct Node {
            id: u32,
            pub parent: Option<Arc<Node>>,
            num_of_descendants: u32,
            num_of_vertices: u64,
            num_of_vertices_in_tree: u64,
            num_of_indices: u64,
            num_of_indices_in_tree: u64,
            // pub local_position: [f32; 3],
            pub model: Option<Model>,
            pub children: Vec<Node>,
        }

        // when a node changes, update the parent

        impl Node {
            pub fn new(
                id: u32,
                scene_ref: Option<&mut Scene>,
                parent: Option<Arc<Node>>,
                model: Option<Model>,
            ) -> Self {
                let (num_of_vertices, num_of_indices, model) = model.map_or((0, 0, None), |mut model| {
                    // DELETE THIS
                    if let Some(scene) = scene_ref {
                        let prev_scene_textures_len = scene.num_of_textures();
                        let mut curr_scene_textures_len = prev_scene_textures_len;

                        let prev_scene_materials_len = scene.num_of_materials();
                        let mut curr_scene_materials_len = prev_scene_materials_len;

                        // update material index and texture index for all model meshes (VERY IMPORTANT)
                        model.textures.iter().for_each(|texture| {
                            // TODO: use a better texture id/key
                            // what happens if textures are the same? how do we update the material index to the correct one?
                            scene
                                .all_textures
                                .insert(curr_scene_textures_len, texture.to_owned());
                            curr_scene_textures_len += 1;
                        });

                        model.meshes.iter_mut().for_each(|mesh| {
                            mesh.update_material_index(prev_scene_textures_len);
                        });

                        model.materials.iter_mut().for_each(|material| {
                            material.update_texture_indexs(prev_scene_textures_len);
                            scene
                                .all_materials
                                .insert(curr_scene_materials_len, material.to_owned());
                            curr_scene_materials_len += 1;
                        });
                    }

                    let num_of_vertices = model.meshes.iter().map(|mesh| mesh.num_of_vertices()).sum();
                    let num_of_indices = model.meshes.iter().map(|mesh| mesh.num_of_indices()).sum();
                    (num_of_vertices, num_of_indices, Some(model))
                });

                Self {
                    id,
                    parent,
                    num_of_descendants: 0,
                    num_of_vertices,
                    num_of_vertices_in_tree: num_of_vertices,
                    num_of_indices,
                    num_of_indices_in_tree: num_of_indices,
                    children: Vec::new(),
                    model,
                }
            }

            pub fn add_node(&mut self, child: Node) {
                // child.parent = Some(Arc::new(self.clone()));
                self.num_of_descendants += 1 + child.num_of_descendants;
                self.num_of_vertices_in_tree += child.num_of_vertices_in_tree;
                self.num_of_indices_in_tree += child.num_of_indices_in_tree;
                self.children.push(child);
            }

            pub fn get_num_of_descendants(&self) -> u32 {
                self.num_of_descendants
            }

            pub fn get_index_and_vertex_buffers(
                &self,
                vertex_buffer_arr: &mut Vec<Subbuffer<[MeshVertex]>>,
                indicies_arr: &mut Vec<u32>,
                command_buffer_builder: &mut AutoCommandBufferBuilder<
                    PrimaryAutoCommandBuffer,
                    Arc<StandardCommandBufferAllocator>,
                >,
            ) {
                if let Some(mode) = &self.model {
                    for mesh in &mode.meshes {
                        vertex_buffer_arr.push(mesh.vertex_buffer.clone());
                        indicies_arr.extend_from_slice(mesh.indices());
                        // command_buffer_builder.bind_vertex_buffers(0, mesh.vertex_buffer.clone());
                        command_buffer_builder.bind_index_buffer(mesh.index_buffer.clone());
                    }
                }

                for child in &self.children {
                    child.get_index_and_vertex_buffers(
                        vertex_buffer_arr,
                        indicies_arr,
                        command_buffer_builder,
                    );
                }
            }

            pub fn id(&self) -> u32 {
                self.id
            }

            pub fn num_of_indices_in_tree(&self) -> u64 {
                self.num_of_indices_in_tree
            }

            pub fn num_of_vertices_in_tree(&self) -> u64 {
                self.num_of_vertices_in_tree
            }

            pub fn num_of_vertices(&self) -> u64 {
                self.num_of_vertices
            }

            pub fn num_of_indices(&self) -> u64 {
                self.num_of_indices
            }
        }

    }

    pub mod scene{
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

    }
    mod texture_manager {

        use crate::assets::texture::Texture;
        use std::collections::HashMap;

        #[derive(Debug)]
        pub struct OrderedTexturesMap {
            map: HashMap<u8, Texture>,
            keys: Vec<u8>,
        }

        impl Default for OrderedTexturesMap {
            fn default() -> Self {
                Self::new()
            }
        }

        impl OrderedTexturesMap {
            pub fn new() -> Self {
                OrderedTexturesMap {
                    map: HashMap::new(),
                    keys: Vec::new(),
                }
            }

            pub fn insert(&mut self, key: u8, value: Texture) {
                let result = self.map.insert(key, value);
                if result.is_none() {
                    self.keys.push(key);
                }
            }

            pub fn get(&self, key: &u8) -> Option<&Texture> {
                self.map.get(key)
            }

            pub fn iter(&self) -> impl Iterator<Item = (&u8, &Texture)> {
                self.keys
                    .iter()
                    .filter_map(move |k| self.map.get_key_value(k))
            }

            pub fn len(&self) -> u8 {
                self.map.len() as u8
            }
        }

    }
}

mod routes {
    use actix_web::web;

    mod health_check {
        use actix_web::{HttpResponse as Resp, Responder};

        use crate::response::ServerMsg;

        pub async fn handler() -> impl Responder {
            Resp::Ok().json(ServerMsg::data("OK"))
        }
    }

    mod lsdk_webhook {
        use actix_web::{http::Method, web, HttpRequest, HttpResponse as Resp, Responder};
        use std::sync::Arc;

        use crate::{
            response::{CommonResponses, ServerMsg},
            state::ServerStateMutex,
            utils,
            webrtc::TurboLivekitConnector,
        };
        use livekit_api::{
            access_token::{self},
            webhooks,
        };
        use log::info;

        pub async fn handler(
            req: HttpRequest,
            server_data: web::Data<ServerStateMutex>,
            body: web::Bytes,
        ) -> impl Responder {
            if req.method().ne(&Method::POST) {
                return Resp::MethodNotAllowed().json(CommonResponses::MethodNotAllowed.json());
            }
            let token_verifier = match access_token::TokenVerifier::new() {
                Ok(i) => i,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };
            let webhook_receiver = webhooks::WebhookReceiver::new(token_verifier);

            let jwt = req
                .headers()
                .get("Authorization")
                .and_then(|hv| hv.to_str().ok())
                .unwrap_or_default()
                .to_string();

            let body = match std::str::from_utf8(&body) {
                Ok(i) => i,
                Err(e) => return Resp::BadRequest().json(ServerMsg::error(e.to_string())),
            };

            let event = match webhook_receiver.receive(body, &jwt) {
                Ok(i) => i,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };

            if event.room.is_some() && event.event == "room_started" {
                info!("ROOM STARTED 🎉");
                let livekit_protocol::Room {
                    name: participant_room_name,
                    max_participants,
                    num_participants,
                    ..
                } = event.room.unwrap();

                if num_participants < max_participants {
                    let mut turbo_webrtc =
                        match TurboLivekitConnector::new(participant_room_name).await {
                            Ok(turbo_webrtc) => turbo_webrtc,
                            Err(e) => {
                                return Resp::InternalServerError()
                                    .json(ServerMsg::error(format!("{e}")))
                            }
                        };

                    let mut server_data = server_data.lock();
                    server_data.turbo_input_tx = Some(turbo_webrtc.get_txt_input_sender());
                    server_data.turbo_livekit_connector_handle = Some(turbo_webrtc);

                    info!("\nSERVER FINISHED PROCESSING ROOM_STARTED WEBHOOK");
                };
            } else {
                info!("received event {}", event.event);
            }

            Resp::Ok().json(ServerMsg::data("Livekit Webhook Successfully Processed"))
        }

        //  images will be in base64
        // stt & images & text go in -> [find a way of batching all this information and sending it to GPT ] -> stream the response from OPENAI to livekit

        // IT SHOULD NEVER TEXT & WRITE AT the same time
    }

    pub fn top_level_routes(cfg: &mut web::ServiceConfig) {
        cfg.service(web::scope("/").service(web::resource("").to(health_check::handler)))
            .service(web::resource("/lsdk-webhook").to(lsdk_webhook::handler));
    }
}

mod utils {
    mod create_bot_token {
        use livekit_api::access_token::{AccessToken, VideoGrants};

        pub fn create_bot_token(room_name: String, ai_name: &str) -> anyhow::Result<String> {
            let api_key = std::env::var("LIVEKIT_API_KEY")?;
            let api_secret = std::env::var("LIVEKIT_API_SECRET")?;

            let ttl = std::time::Duration::from_secs(60 * 5); // 10 minutes (in sync with frontend)
            Ok(
                AccessToken::with_api_key(api_key.as_str(), api_secret.as_str())
                    .with_ttl(ttl)
                    .with_identity(ai_name)
                    .with_name(ai_name)
                    .with_grants(VideoGrants {
                        room: room_name,
                        room_list: true,
                        room_join: true,
                        room_admin: true,
                        can_publish: true,
                        room_record: true,
                        can_subscribe: true,
                        can_publish_data: true,
                        can_update_own_metadata: true,
                        ..Default::default()
                    })
                    .to_jwt()?,
            )
        }
    }

    pub use create_bot_token::*;
}

mod track_pub {
    use std::sync::Arc;

    use livekit::{
        options::{TrackPublishOptions, VideoCodec},
        publication::LocalTrackPublication,
        track::{LocalAudioTrack, LocalTrack, LocalVideoTrack, TrackSource},
        webrtc::{
            audio_source::native::NativeAudioSource,
            prelude::{AudioSourceOptions, RtcAudioSource},
            video_source::{native::NativeVideoSource, RtcVideoSource},
        },
        Room, RoomError,
    };

    use crate::stt::STT;

    pub struct TracksPublicationData {
        pub video_src: NativeVideoSource,
        pub video_pub: LocalTrackPublication,
        pub audio_src: NativeAudioSource,
        pub audio_pub: LocalTrackPublication,
    }
    const BOT_NAME: &str = "donut";

    pub async fn publish_tracks(room: Arc<Room>) -> Result<TracksPublicationData, RoomError> {
        let audio_src = NativeAudioSource::new(
            AudioSourceOptions::default(),
            STT::SAMPLE_RATE,
            STT::NUM_OF_CHANNELS,
        );
        let audio_track = LocalAudioTrack::create_audio_track(
            BOT_NAME,
            RtcAudioSource::Native(audio_src.clone()),
        );

        // TODO: Remove from here and import from Turbo. Resolution{} ?
        let (width, height) = (1920, 1080);
        let video_src = NativeVideoSource::new(livekit::webrtc::video_source::VideoResolution {
            width,
            height,
        });
        let video_track = LocalVideoTrack::create_video_track(
            BOT_NAME,
            RtcVideoSource::Native(video_src.clone()),
        );

        let video_publication = room
            .local_participant()
            .publish_track(
                LocalTrack::Video(video_track),
                TrackPublishOptions {
                    source: TrackSource::Camera,
                    video_codec: VideoCodec::VP8,
                    ..Default::default()
                },
            )
            .await;
        let audio_publication = room
            .local_participant()
            .publish_track(
                LocalTrack::Audio(audio_track),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    ..Default::default()
                },
            )
            .await;

        let video_pub = video_publication?;
        let audio_pub = audio_publication?;
        Ok(TracksPublicationData {
            video_src,
            video_pub,
            audio_src,
            audio_pub,
        })
    }
}

mod stt {
    use async_trait::async_trait;
    use bytes::{BufMut, Bytes, BytesMut};
    use deepgram::Deepgram;
    use ezsockets::{
        client::ClientCloseMode, Client, ClientConfig, CloseFrame, MessageSignal, MessageStatus,
        RawMessage, SocketConfig, WSError,
    };
    use futures::StreamExt;
    use livekit::webrtc::audio_stream::native::NativeAudioStream;
    use log::{error, info};
    use parking_lot::Mutex;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Map, Value};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio::sync::mpsc::UnboundedSender;

    #[derive(Clone)]
    pub struct STT {
        ws_client: Client<WSClient>,
    }

    struct WSClient {
        to_gpt: tokio::sync::mpsc::UnboundedSender<String>,
    }

    #[async_trait]
    impl ezsockets::ClientExt for WSClient {
        type Call = ();

        async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
            let data: Value = serde_json::from_str(&text)?;
            let transcript_details = data["channel"]["alternatives"][0].clone();

            info!("\n\n\n received message from deepgram: {data}");
            info!("\n\n\n received message from deepgram: {transcript_details}");
            // if transcript_details!= Value::Null {
            //     self.to_gpt.send(transcript_details.to_string())?;
            // }
            Ok(())
        }

        async fn on_binary(&mut self, bytes: Vec<u8>) -> Result<(), ezsockets::Error> {
            info!("received bytes: {bytes:?}");
            Ok(())
        }

        async fn on_call(&mut self, call: Self::Call) -> Result<(), ezsockets::Error> {
            info!("DEEPGRAM ON CALL: {call:?}");
            let () = call;
            Ok(())
        }
        async fn on_connect(&mut self) -> Result<(), ezsockets::Error> {
            info!("DEEPGRAM CONNECTED 🎉");
            Ok(())
        }

        async fn on_connect_fail(
            &mut self,
            _error: WSError,
        ) -> Result<ClientCloseMode, ezsockets::Error> {
            info!("DEEPGRAM connection FAIL 💔");
            Ok(ClientCloseMode::Reconnect)
        }

        async fn on_close(
            &mut self,
            _frame: Option<CloseFrame>,
        ) -> Result<ClientCloseMode, ezsockets::Error> {
            info!("DEEPGRAM connection CLOSE 💔");
            Ok(ClientCloseMode::Reconnect)
        }

        async fn on_disconnect(&mut self) -> Result<ClientCloseMode, ezsockets::Error> {
            info!("DEEPGRAM disconnect 💔");
            Ok(ClientCloseMode::Reconnect)
        }
    }

    impl STT {
        pub const LATENCY_FRAMES: f32 = (Self::LATENCY_MS / 1_000.0) * Self::SAMPLE_RATE_F32;
        // Uses a delay of `LATENCY_MS` milliseconds in case the default input and output streams are not precisely synchronised
        pub const LATENCY_MS: f32 = 5000.0;
        pub const LATENCY_SAMPLES: u32 = Self::LATENCY_FRAMES as u32 * Self::NUM_OF_CHANNELS;
        pub const NUM_ITERS: usize = 2;
        pub const NUM_ITERS_SAVED: usize = 2;
        pub const NUM_OF_CHANNELS: u32 = 1;
        pub const SAMPLE_RATE: u32 = 44100; //1600
        pub const SAMPLE_RATE_F32: f32 = Self::SAMPLE_RATE as f32;
        pub const SAMPLING_FREQ: f32 = Self::SAMPLE_RATE_F32 / 2.0;

        const MIN_AUDIO_MS_CHUNK: u64 = 25;

        pub async fn new(
            gpt_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
        ) -> anyhow::Result<Self> {
            let deepgram_api_key = std::env::var("DEEPGRAM_API_KEY")
                .expect("The DEEPGRAM_API_KEY env variable is required!");

            let config = ClientConfig::new("wss://api.deepgram.com/v1/listen")
                .socket_config(SocketConfig {
                    heartbeat: Duration::from_secs(8),
                    timeout: Duration::from_secs(30 * 60), // 30 minutes
                    heartbeat_ping_msg_fn: Arc::new(|_t: Duration| {
                        RawMessage::Text(r#"{ "type": "KeepAlive" }"#.into())
                    }),
                })
                .header("authorization", &format!("token {}", deepgram_api_key))
                .query_parameter("encoding", "linear16")
                .query_parameter("sample_rate", &Self::SAMPLE_RATE.to_string())
                .query_parameter("channels", &Self::NUM_OF_CHANNELS.to_string())
                .query_parameter("model", "2-conversationalai")
                .query_parameter("smart_format", "true")
                .query_parameter("filler_words", "true")
                .query_parameter("version", "latest")
                .query_parameter("tier", "nova");

            let (ws_client, future) = ezsockets::connect(
                |_client| WSClient {
                    to_gpt: gpt_input_tx,
                },
                config,
            )
            .await;

            Ok(Self { ws_client })
        }
        fn send(&self, bytes: impl Into<Vec<u8>>) -> anyhow::Result<MessageStatus> {
            let signal = self.ws_client.binary(bytes)?;
            Ok(signal.status())
        }
    }

    pub async fn transcribe(stt_client: STT, mut audio_stream: NativeAudioStream) {
        // let mut curr_audio_len = 0.0_f32; // in ms

        let mut starting_time = Instant::now();
        let mut audio_buffer: Vec<u8> = Vec::new();

        while let Some(frame) = audio_stream.next().await {
            // curr_audio_len += (num_of_sample as u32 / frame.sample_rate) as f32 /1000.0;

            let num_of_samples = frame.data.len();

            let mut bytes = BytesMut::with_capacity(num_of_samples * 2);
            frame
                .data
                .iter()
                .for_each(|sample| bytes.put_i16_le(*sample));

            audio_buffer.extend(bytes);

            if starting_time.elapsed() > Duration::from_millis(STT::MIN_AUDIO_MS_CHUNK) {
                match stt_client.send(audio_buffer.clone()) {
                    Ok(status) => info!("Sent audio to deegram | Msg status {status:?}"),
                    Err(e) => error!("Error sending audio bytes to deepgram ws {e}"),
                };

                starting_time = Instant::now();
                audio_buffer.clear();
            }
        }
    }

    impl Drop for STT {
        fn drop(&mut self) {
            if let Err(e) = self.send([]) {
                error!("Error shutting down STT  / Deepgram connection | Reason - {e}");
            };
        }
    }
}

mod room_events {
    use std::sync::Arc;

    use chrono::TimeZone;
    use futures::StreamExt;
    use livekit::{
        track::RemoteTrack,
        webrtc::{
            audio_stream::native::NativeAudioStream, video_stream::native::NativeVideoStream,
        },
        DataPacketKind, RoomEvent,
    };
    use log::{error, info, warn};
    use parking_lot::{Mutex, MutexGuard, RawMutex};
    use serde::{Deserialize, Serialize};

    use crate::stt::{transcribe, STT};

    #[derive(Serialize, Deserialize)]
    struct RoomText {
        message: String,
        timestamp: i64,
    }

    pub async fn handle_room_events(
        gpt_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
        stt_client: STT,
        mut room_events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    ) -> anyhow::Result<()> {
        while let Some(event) = room_events.recv().await {
            match event {
                RoomEvent::TrackSubscribed {
                    track,
                    publication: _,
                    participant: _user,
                } => match track {
                    RemoteTrack::Audio(audio_track) => {
                        let audio_rtc_track = audio_track.rtc_track();
                        let audio_stream = NativeAudioStream::new(audio_rtc_track);
                        let stt_client_for_thread = stt_client.clone();
                        tokio::spawn(transcribe(stt_client_for_thread, audio_stream));
                    }
                    RemoteTrack::Video(video_track) => {
                        let video_rtc_track = video_track.rtc_track();
                        let video_stream = NativeVideoStream::new(video_rtc_track);
                        tokio::spawn(video_stream_handler(video_stream));
                    }
                },
                RoomEvent::DataReceived {
                    payload,
                    kind,
                    participant: _user,
                } => {
                    if kind == DataPacketKind::Reliable {
                        if let Some(payload) = payload.as_ascii() {
                            let room_text: serde_json::Result<RoomText> =
                                serde_json::from_str(payload.as_str());
                            match room_text {
                                Ok(room_text) => {
                                    if let Err(e) =
                                        gpt_input_tx.send(format!("{} ", room_text.message))
                                    {
                                        error!("Couldn't send the text to gpt {e}")
                                    };
                                }
                                Err(e) => {
                                    warn!("Couldn't deserialize room text. {e:#?}");
                                }
                            }

                            info!("text from room {:#?}", payload.as_str());
                        }
                    }
                }
                // RoomEvents::TrackMuted {} =>{

                // }
                _ => info!("incoming event {:?}", event),
            }
        }
        Ok(())
    }

    async fn video_stream_handler(mut video: NativeVideoStream) {
        let mut counter = 0_u8;
        let max_fps = 10;

        while let Some(frame) = video.next().await {
            if counter % max_fps == 0 {
                info!("video frame info - {frame:#?}");
            }

            counter = (counter + 1) % max_fps;
        }
    }
}
mod response {
    use std::fmt::Debug;

    use log::warn;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    pub struct ServerMsg<T> {
        data: Option<T>,
        error: Option<String>,
    }

    impl<T: ToString> ServerMsg<T> {
        pub fn data(data: T) -> Self {
            Self {
                data: Some(data),
                error: None,
            }
        }

        pub fn error(error: T) -> Self {
            let err_msg = error.to_string();
            warn!("Server error. {err_msg:?}");
            Self {
                data: None,
                error: Some(err_msg),
            }
        }
    }

    #[derive(Serialize)]
    pub struct DefaultGameResponse {
        pub board: Vec<i8>,
        pub state: String,
    }
    #[derive(Deserialize)]
    pub struct PlayDetails {
        pub position: u8,
    }

    pub enum CommonResponses {
        MethodNotAllowed,
    }

    impl CommonResponses {
        pub fn json(&self) -> ServerMsg<String> {
            match self {
                CommonResponses::MethodNotAllowed => {
                    ServerMsg::error("Method not allowed".to_string())
                }
            }
        }
    }
}

mod gpt {
    use std::time::{Duration, Instant};

    use async_openai::{
        config::OpenAIConfig,
        types::{
            ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
            CreateChatCompletionRequestArgs,
        },
        Client,
    };
    use futures::StreamExt;
    use log::{error, info, warn};
    use tokio::{sync::mpsc, time};

    use crate::{stt::STT, tts::TTS};

    /// Stream text chunks to gpt as it's being generated, with <1s latency.
    /// Note: if chunks don't end with space or punctuation (" ", ".", "?", "!"),
    /// the stream will wait for more text.
    /// Used during input streaming to chunk text blocks and set last char to space
    pub async fn gpt(
        mut text_input_rx: mpsc::UnboundedReceiver<String>,
        openai_client: Client<OpenAIConfig>,
        mut tts_client: TTS,
    ) -> anyhow::Result<()> {
        let splitters = [
            '.', ',', '?', '!', ';', ':', '—', '-', '(', ')', '[', ']', '}', ' ',
        ];

        let mut txt_buffer = String::new();
        let mut tts_buffer = String::new();

        let mut req_args = CreateChatCompletionRequestArgs::default();
        let openai_req = req_args.model("gpt-3.5-turbo").max_tokens(512u16);

        // let text_latency = Duration::from_millis(500);
        while let Some(chunk) = text_input_rx.recv().await {
            txt_buffer.push_str(&chunk);
            if ends_with_splitter(&splitters, &txt_buffer) {
                let request = openai_req
                    .messages([ChatCompletionRequestUserMessageArgs::default()
                        .content(ChatCompletionRequestUserMessageContent::Text(
                            txt_buffer.clone(),
                        ))
                        .build()?
                        .into()])
                    .build()?;

                let mut gpt_resp_stream = openai_client.chat().create_stream(request).await?;
                while let Some(result) = gpt_resp_stream.next().await {
                    match result {
                        Ok(response) => {
                            for chat_choice in response.choices {
                                if let Some(content) = chat_choice.delta.content {
                                    tts_buffer.push_str(&content);
                                    if ends_with_splitter(&splitters, &tts_buffer) {
                                        if let Err(e) = tts_client.send(tts_buffer.clone()) {
                                            error!(
                                                "Coudln't send gpt text chunk to tts channel - {e}"
                                            );
                                        } else {
                                            tts_buffer.clear();
                                        };
                                    }
                                };
                            }
                        }
                        Err(err) => {
                            warn!("chunk error: {err:#?}");
                        }
                    }
                }
                txt_buffer.clear();
            } else if !txt_buffer.ends_with(' ') {
                txt_buffer.push(' ');
            }
        }
        Ok(())
    }

    fn ends_with_splitter(splitters: &[char], chunk: &str) -> bool {
        !chunk.is_empty()
            && chunk != " "
            && splitters.iter().any(|&splitter| chunk.ends_with(splitter))
    }
}

mod core {
    #![allow(unused_parens, non_snake_case)]
    use anyhow::{Context, Result};
    use log::{info, warn};
    use std::sync::Arc;
    use vulkano::{
        device::{
            physical::{PhysicalDevice, PhysicalDeviceType},
            Device, DeviceCreateInfo, DeviceExtensions, Features, Properties, Queue,
            QueueCreateInfo, QueueFlags,
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
            let gfx_queue = queues
                .next()
                .expect("failed to get first queue in iterator");

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
                    let Properties {
                        device_name,
                        device_type,
                        ..
                    } = &p.properties();
                    info!(
                        "- {} | {:?} | Vulkan v{:?}",
                        device_name,
                        device_type,
                        p.api_version()
                    );
                    p.api_version() >= Version::V1_3
                        || p.supported_extensions().khr_dynamic_rendering
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
}

mod tts {
    use anyhow::bail;
    use async_trait::async_trait;
    use base64::{engine::general_purpose, Engine as _};
    use bytes::{Buf, BufMut, Bytes, BytesMut};
    use deepgram::Deepgram;
    use ezsockets::{
        client::ClientCloseMode, Client, ClientConfig, CloseFrame, MessageSignal, MessageStatus,
        RawMessage, SocketConfig, WSError,
    };
    use futures::StreamExt;
    use livekit::webrtc::{
        audio_frame::AudioFrame, audio_source::native::NativeAudioSource,
        audio_stream::native::NativeAudioStream, native::audio_resampler,
    };
    use log::{error, info};
    use parking_lot::Mutex;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Map, Value};
    use std::{
        env,
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio::sync::mpsc::UnboundedSender;

    use crate::stt::STT;

    #[derive(Serialize)]
    struct VoiceSettings {
        stability: f32,
        similarity_boost: bool,
    }

    #[derive(Serialize)]
    struct BOSMessage<'a> {
        text: &'a str,
        voice_settings: VoiceSettings,
    }

    #[derive(Serialize)]
    struct EOSMessage<'a> {
        text: &'a str,
    }

    #[derive(Serialize)]
    struct RegularMessage {
        text: String,
        try_trigger_generation: bool,
    }

    struct NormalizedAlignment {
        char_start_times_ms: Vec<u8>,
        chars_durations_ms: Vec<u8>,
        chars: Vec<char>,
    }
    struct ElevenLabs {
        audio: String,
        isFinal: bool,
        normalizedAlignment: NormalizedAlignment,
    }

    #[derive(Clone)]
    pub struct TTS {
        ws_client: Option<Client<WSClient>>,
        pub started: bool,
        eleven_labs_api_key: String,
    }

    struct WSClient {
        audio_src: NativeAudioSource,
        tts_client_ref: Arc<Mutex<TTS>>,
    }

    fn vec_u8_to_vec_i16(input: Vec<u8>) -> Vec<i16> {
        // Ensure that the input Vec<u8> has an even number of elements
        if input.len() % 2 != 0 {
            panic!("Input Vec<u8> must have an even number of elements");
        }

        input
            .chunks(2)
            .map(|chunk| {
                // Convert each pair of u8 to one i16
                // Little-endian order: The first byte is the least significant
                i16::from_le_bytes([chunk[0], chunk[1]])
            })
            .collect()
    }

    #[async_trait]
    impl ezsockets::ClientExt for WSClient {
        type Call = ();

        async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
            let data: Value = serde_json::from_str(&text)?;
            let transcript_details = data["audio"].clone();

            if transcript_details != Value::Null {
                let data = general_purpose::STANDARD_NO_PAD
                    .decode(transcript_details.as_str().unwrap())?;

                const FRAME_DURATION: Duration = Duration::from_millis(500); // Write 0.5s of audio at a time
                let ms = FRAME_DURATION.as_millis() as u32;

                let num_channels = self.audio_src.num_channels();
                let sample_rate = self.audio_src.sample_rate();
                let num_samples = (sample_rate / 1000 * ms) as usize;
                let samples_per_channel = num_samples as u32;

                // let mut resampler = audio_resampler::AudioResampler::default();
                // resampler.

                let audio_frame = AudioFrame {
                    data: vec_u8_to_vec_i16(data).into(),
                    num_channels,
                    sample_rate,
                    samples_per_channel,
                };

                self.audio_src.capture_frame(&audio_frame).await?;
            } else {
                error!("received message from eleven labs: {text}");
            }

            Ok(())
        }

        async fn on_binary(&mut self, bytes: Vec<u8>) -> Result<(), ezsockets::Error> {
            info!("received bytes: {bytes:?}");
            Ok(())
        }

        async fn on_call(&mut self, call: Self::Call) -> Result<(), ezsockets::Error> {
            info!("ELEVEN LABS WTF");
            let () = call;
            Ok(())
        }

        async fn on_connect(&mut self) -> Result<(), ezsockets::Error> {
            info!("ELEVEN LABS CONNECTED 🎉");
            Ok(())
        }

        async fn on_connect_fail(
            &mut self,
            _error: WSError,
        ) -> Result<ClientCloseMode, ezsockets::Error> {
            info!("ELEVEN LABS connection FAIL 💔");
            Ok(ClientCloseMode::Reconnect)
        }

        async fn on_close(
            &mut self,
            _frame: Option<CloseFrame>,
        ) -> Result<ClientCloseMode, ezsockets::Error> {
            info!("ELEVEN LABS connection CLOSE 💔");
            let mut tts = self.tts_client_ref.lock();
            tts.started = false;
            Ok(ClientCloseMode::Reconnect)
        }

        async fn on_disconnect(&mut self) -> Result<ClientCloseMode, ezsockets::Error> {
            info!("ELEVEN LABS disconnect 💔");
            Ok(ClientCloseMode::Reconnect)
        }
    }

    impl TTS {
        pub fn new() -> anyhow::Result<Self> {
            let eleven_labs_api_key = std::env::var("ELEVENLABS_API_KEY")
                .expect("The ELEVENLABS_API_KEY env variable is required!");

            Ok(Self {
                ws_client: None,
                started: false,
                eleven_labs_api_key,
            })
        }

        pub async fn setup_ws_client(
            &mut self,
            audio_src: NativeAudioSource,
        ) -> anyhow::Result<()> {
            let ws_client = self.connect_ws_client(audio_src).await?;
            self.started = true;
            self.ws_client = Some(ws_client);
            Ok(())
        }

        async fn connect_ws_client(
            &mut self,
            audio_src: NativeAudioSource,
        ) -> anyhow::Result<Client<WSClient>> {
            let voice_id = "L1oawlP7wF6KPWjLuHcF";
            let model = "eleven_monolingual_v1";

            let url = url::Url::parse(&format!(
                "wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input?model_id={model}"
            ))
            .unwrap();

            let config = ClientConfig::new(url)
                .socket_config(SocketConfig {
                    heartbeat: Duration::from_secs(10),
                    timeout: Duration::from_secs(30 * 60), // 30 minutes
                    heartbeat_ping_msg_fn: Arc::new(|_t: Duration| {
                        RawMessage::Text(
                            serde_json::to_string(&RegularMessage {
                                text: "  ".to_string(),
                                try_trigger_generation: true,
                            })
                            .unwrap(),
                        )
                    }),
                })
                .header("xi-api-key", &self.eleven_labs_api_key)
                .header("Content-Type", "application/json")
                .header("optimize_streaming_latency", "3")
                .header("output_format", "pcm_16000");

            let (ws_client, future) = ezsockets::connect(
                |_client| WSClient {
                    audio_src,
                    tts_client_ref: Arc::new(Mutex::new(self.clone())),
                },
                config,
            )
            .await;

            ws_client.text(serde_json::to_string(&BOSMessage {
                text: " ",
                voice_settings: VoiceSettings {
                    stability: 0.5,
                    similarity_boost: false,
                },
            })?)?;
            Ok(ws_client)
        }

        pub fn start(&mut self) -> anyhow::Result<()> {
            self.started = true;
            self.send(" ".to_string())?;
            Ok(())
        }

        pub fn send(&mut self, msg: String) -> anyhow::Result<MessageStatus> {
            let msg = match msg.as_str() {
                "" => serde_json::to_string(&EOSMessage { text: "" }),
                " " => serde_json::to_string(&BOSMessage {
                    text: " ",
                    voice_settings: VoiceSettings {
                        stability: 0.5,
                        similarity_boost: false,
                    },
                }),
                msg => serde_json::to_string(&RegularMessage {
                    text: format!("{msg} "),
                    try_trigger_generation: true,
                }),
            };
            let msg = msg?;

            if !self.started {
                self.start()?;
            }

            if self.ws_client.as_ref().is_none() {
                bail!("ws_client is none");
            }

            info!("sending to eleven labs {msg}");

            Ok(self.ws_client.as_ref().unwrap().text(msg)?.status())
        }
    }

    impl Drop for TTS {
        fn drop(&mut self) {
            info!("DROPPING TTS");
            if let Err(e) = self.send("".to_owned()) {
                error!("Error shutting down TTS  / Eleven Labs connection | Reason - {e}");
            };
        }
    }
}
mod state {
    use log::{error, info};

    use crate::webrtc::TurboLivekitConnector;

    pub type ServerStateMutex = parking_lot::Mutex<ServerState>;

    #[derive(Default)]
    pub struct ServerState {
        pub turbo_input_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
        pub turbo_livekit_connector_handle: Option<TurboLivekitConnector>,
    }

    impl ServerState {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl Drop for ServerState {
        fn drop(&mut self) {
            if let Some(turbo_input_tx) = self.turbo_input_tx.take() {
                match turbo_input_tx.send("Goodbye".to_owned()) {
                    Ok(_) => info!("Turbo Renderer should be exiting..."),
                    Err(e) => error!("Error closing renderer: {e}"),
                };
            }

            if let Some(render_thread_handle) = self.turbo_livekit_connector_handle.take() {
                drop(render_thread_handle);
            }
        }
    }
}

mod turbo {
    use std::{
        sync::{mpsc::Receiver, Arc},
        time::{Duration, Instant},
    };

    use anyhow::{bail, Result};

    use crate::{
        core::Engine,
        scene::{camera::Camera, scene::Scene},
    };

    use image::{ImageBuffer, Rgba};
    use livekit::webrtc::{
        audio_source::native::NativeAudioSource,
        native::yuv_helper,
        video_frame::{I420Buffer, VideoFrame, VideoRotation},
        video_source::native::NativeVideoSource,
    };
    use log::{error, info, warn};
    use parking_lot::Mutex;
    use vulkano::{command_buffer::PrimaryCommandBufferAbstract, sync::GpuFuture};

    const PIXEL_SIZE: usize = 4;
    const FB_WIDTH: usize = 1920;
    const FB_HEIGHT: usize = 1080;

    #[derive(Clone)]
    struct FrameData {
        image: ImageBuffer<Rgba<u8>, Vec<u8>>,
        framebuffer: Arc<Mutex<Vec<u8>>>,
        video_frame: Arc<Mutex<VideoFrame<I420Buffer>>>,
    }

    pub struct Turbo {
        camera: Camera,
        engine: Engine,
        scene: Scene,
        target_fps: f32,
    }

    impl Turbo {
        pub fn new() -> Result<Self> {
            let engine = Engine::new()?;
            let scene = Scene::new(engine.get_vkdevice(), engine.gfx_queue_family_index())?;
            let camera = Camera::new().update_aspect_ratio_from_scene(scene.width_height());

            let target_fps = 30.0_f32;

            Ok(Self {
                engine,
                scene,
                camera,
                target_fps,
            })
        }

        pub fn load_basic_scene(mut self) -> Result<Self> {
            let donut_model = self.scene.load_gltf("oreo_donut/scene.gltf")?; // , adamHead/adamHead.gltf
            let donut_node = self.scene.create_node(Some(donut_model));
            self.scene.add_node(donut_node);
            Ok(self)
        }

        pub fn get_scene_width_height(&self) -> [u32; 2] {
            self.scene.width_height()
        }

        pub fn get_fps(&self) -> f32 {
            self.target_fps
        }

        fn get_fps_i32(&self) -> u32 {
            self.target_fps as u32
        }

        pub async fn render(
            &mut self,
            // render_input_receiver: Receiver<String>,
            livekit_rtc_src: NativeVideoSource,
        ) -> Result<()> {
            let (pipeline, framebuffer) = self.scene.new_gfx_pipeline_and_frame_buffer()?;
            let now = Instant::now();
            let rotation_start = now;
            let mut num_of_frames = 0;
            let fps = self.get_fps_i32();

            let mut interval =
                tokio::time::interval(Duration::from_millis(1000 / self.target_fps as u64));

            let [w, h] = self.scene.width_height();
            let default_image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(
                w,
                h,
                vec![0u8; FB_WIDTH * FB_HEIGHT * 4],
            )
            .unwrap();
            let mut frame_data = FrameData {
                image: default_image,
                framebuffer: Arc::new(Mutex::new(vec![0u8; FB_WIDTH * FB_HEIGHT * 4])),
                video_frame: Arc::new(Mutex::new(VideoFrame {
                    rotation: VideoRotation::VideoRotation0,
                    buffer: I420Buffer::new(FB_WIDTH as u32, FB_HEIGHT as u32),
                    timestamp_us: 0,
                })),
            };

            loop {
                // if let Some(input) = get_user_input(&render_input_receiver)? {
                //     let input = input.as_str();
                //     if is_exit_cmd(input) {
                //         return Ok(());
                //     }
                //     self.scene.handle_input(input);
                // }

                let elapsed_time = rotation_start.elapsed();

                self.camera.update_rotation(
                    elapsed_time.as_secs_f64() as f32
                        + elapsed_time.subsec_nanos() as f32 / 20_000_000_000.0,
                );

                self.scene
                    .update_camera_subbuffer_allocator(self.camera.format_to_subbuffer_data())?;

                let command_buffer = self.scene.build_primary_cmd_buffer(
                    &pipeline,
                    framebuffer.clone(),
                    self.camera.get_model_matrix(),
                )?;

                let finished = command_buffer.execute(self.engine.get_gfx_queue())?;

                finished.then_signal_fence_and_flush()?.wait(None)?;

                let buffer_content = self.scene.img_buffer_content()?.to_vec();

                let [w, h] = self.scene.width_height();

                let image = match ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w, h, buffer_content) {
                    Some(img) => img,
                    None => bail!("Failed to create image buffer"),
                };

                frame_data.image = image;

                if let Err(e) = tokio::task::spawn_blocking({
                    let frame_data = frame_data.clone();
                    let source = livekit_rtc_src.clone();
                    move || {
                        let img_vec = frame_data.image.as_raw();
                        let mut framebuffer = frame_data.framebuffer.lock();
                        let mut video_frame = frame_data.video_frame.lock();
                        let i420_buffer = &mut video_frame.buffer;

                        let (stride_y, stride_u, stride_v) = i420_buffer.strides();
                        let (data_y, data_u, data_v) = i420_buffer.data_mut();

                        framebuffer.clone_from_slice(img_vec);

                        yuv_helper::abgr_to_i420(
                            &framebuffer,
                            (FB_WIDTH * PIXEL_SIZE) as u32,
                            data_y,
                            stride_y,
                            data_u,
                            stride_u,
                            data_v,
                            stride_v,
                            FB_WIDTH as i32,
                            FB_HEIGHT as i32,
                        );

                        source.capture_frame(&*video_frame);
                    }
                })
                .await
                {
                    error!("Error sending video frame to livekit {e}");
                };

                interval.tick().await;

                // if num_of_frames % fps == 0 {
                //     info!(
                //         "[ Frame time after limiter : {:.3} ms ]",
                //         curr_frame_time.elapsed().as_secs_f64() * 1000.0
                //     );
                // }

                num_of_frames = (num_of_frames + 1) % fps;
            }
        }
    }

    fn get_user_input(input_revr: &Receiver<String>) -> Result<Option<String>> {
        match input_revr.try_recv() {
            Ok(input) => Ok(Some(input.clone())),
            Err(err) => match err {
                std::sync::mpsc::TryRecvError::Empty => Ok(None),
                std::sync::mpsc::TryRecvError::Disconnected => {
                    bail!("Render input channel disconnected")
                }
            },
        }
    }

    fn is_exit_cmd(input: &str) -> bool {
        input == "exit"
            || input == "quit"
            || input == "q"
            || input == "bye"
            || input == "goodbye"
            || input == "ciao"
            || input == "adios"
    }

    impl Drop for Turbo {
        fn drop(&mut self) {
            warn!("DROPPED Turbo");
        }
    }

    unsafe impl Send for Turbo {}
    unsafe impl Sync for Turbo {}
}

mod webrtc {
    use std::sync::Arc;

    use anyhow::Result;
    use async_openai::{config::OpenAIConfig, Client as OPENAI_CLIENT};
    use livekit::{publication::LocalTrackPublication, Room};

    use livekit as lsdk;
    use log::{error, info, warn};
    use lsdk::RoomError;
    use parking_lot::Mutex;
    use tokio::{
        sync::mpsc::{Receiver, UnboundedReceiver},
        task::JoinHandle,
    };

    use crate::{
        gpt::gpt,
        room_events::handle_room_events,
        stt::STT,
        track_pub::{publish_tracks, TracksPublicationData},
        tts::TTS,
        turbo::Turbo,
        utils,
    };

    pub struct TurboLivekitConnector {
        room: Arc<Room>,
        text_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
        cmd_input_sender: std::sync::mpsc::Sender<String>,
        room_event_handle: JoinHandle<Result<()>>,
        video_pub: LocalTrackPublication,
        audio_pub: LocalTrackPublication,
        gpt_thread_handle: JoinHandle<()>,
        render_thread_handle: Option<JoinHandle<()>>,
    }

    const BOT_NAME: &str = "talking_donut";

    impl TurboLivekitConnector {
        pub async fn new(participant_room_name: String) -> Result<Self> {
            // ************** REQUIRED ENV VARS **************
            let open_ai_org_id = std::env::var("OPENAI_ORG_ID").expect("OPENAI_ORG_ID must be");
            let lvkt_url = std::env::var("LIVEKIT_WS_URL").expect("LIVEKIT_WS_URL is not set");

            // ************** CONNECT TO ROOM **************
            let lvkt_token = utils::create_bot_token(participant_room_name, BOT_NAME)?;
            let room_options = lsdk::RoomOptions {
                ..Default::default()
            };
            let (room, room_events) =
                lsdk::Room::connect(&lvkt_url, &lvkt_token, room_options).await?;
            info!("Established connection with room. ID -> [{}]", room.name());
            let room = Arc::new(room);

            // ************** CREATE MESSAGING CHANNELS **************
            let (cmd_input_sender, cmd_input_receiver) = std::sync::mpsc::channel::<String>();
            let (gpt_input_tx, gpt_input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let (to_voice_tx, from_gpt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

            // ************** SETUP OPENAI, TTS, & STT **************
            let TracksPublicationData {
                video_pub,
                video_src,
                audio_src,
                audio_pub,
            } = publish_tracks(room.clone()).await?;

            let openai_client =
                OPENAI_CLIENT::with_config(OpenAIConfig::new().with_org_id(open_ai_org_id));
            let mut turbo = Turbo::new()?.load_basic_scene()?;
            let stt_cleint = STT::new(gpt_input_tx.clone()).await?;
            let mut tts_client = TTS::new()?;
            tts_client.setup_ws_client(audio_src).await?;

            // ************** CREATE THREADS TO KICK THINGS OFF **************
            let room_event_handle = tokio::spawn(handle_room_events(
                gpt_input_tx.clone(),
                stt_cleint,
                room_events,
            ));

            // let tts_receiver_handle = tokio::spawn(tts_receiver(from_gpt_rx, tts_client_for_receiver));

            // let tts_thread_handle = tokio::spawn(tts.transcribe(main_input_rx));

            let gpt_thread_handle = tokio::spawn(async {
                if let Err(e) = gpt(gpt_input_rx, openai_client, tts_client).await {
                    error!("GPT thread exited with error: {e}");
                }
            });

            let render_thread_handle = tokio::spawn(async move {
                if let Err(e) = turbo.render(video_src).await {
                    error!("Turbo graphics render thread exited with error: {e}");
                }
            });

            Ok(Self {
                room,
                text_input_tx: gpt_input_tx,
                audio_pub,
                video_pub,
                room_event_handle,
                cmd_input_sender,
                gpt_thread_handle,
                render_thread_handle: Some(render_thread_handle),
            })
        }

        pub fn get_thread_handle(&mut self) -> JoinHandle<()> {
            self.render_thread_handle
                .take()
                .expect("render thread handle should not be None")
        }

        pub fn get_txt_input_sender(&mut self) -> tokio::sync::mpsc::UnboundedSender<String> {
            self.text_input_tx.clone()
        }

        async fn shutdown(&mut self) -> Result<(), RoomError> {
            self.room.close().await
        }
    }

    impl Drop for TurboLivekitConnector {
        fn drop(&mut self) {
            if let Err(e) = futures::executor::block_on(self.shutdown()) {
                warn!("Error shutting down turbo webrtc | {e}");
            };
        }
    }

    unsafe impl Send for TurboLivekitConnector {}
    unsafe impl Sync for TurboLivekitConnector {}
}

// ACTIX SERVER
#[tokio::main]
async fn main() -> std::io::Result<()> {
    // tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    std::env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY must be set");
    std::env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET must be set");
    let port = env::var("PORT")
        .unwrap_or_else(|_| "6669".to_string())
        .parse::<u16>()
        .expect("PORT couldn't be set");

    let mut formatted_builder = pretty_env_logger::formatted_builder();
    let pretty_env_builder = formatted_builder
        .filter_module("lkgpt", log::LevelFilter::Info)
        .filter_module("actix_server", log::LevelFilter::Info)
        .filter_module("actix_web", log::LevelFilter::Info);
    if cfg!(target_os = "unix") {
        pretty_env_builder.filter_module("livekit", log::LevelFilter::Info);
    }
    pretty_env_builder.init();

    let server_data = Data::new(parking_lot::Mutex::new(state::ServerState::new()));

    info!("starting HTTP server on port {port}");

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::new("IP - %a | Time - %D ms"))
            .wrap(middleware::DefaultHeaders::new().add(("Content-Type", "application/json")))
            .app_data(server_data.clone())
            .configure(routes::top_level_routes)
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
