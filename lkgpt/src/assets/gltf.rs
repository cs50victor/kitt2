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
