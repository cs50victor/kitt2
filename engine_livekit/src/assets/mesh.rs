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
            BufferCreateInfo { usage: BufferUsage::VERTEX_BUFFER, ..Default::default() },
            AllocationCreateInfo { usage: MemoryUsage::Upload, ..Default::default() },
            vertices.clone(),
        )
        .context("failed to create vertex buffer")?;

        let index_buffer = Buffer::from_iter(
            memory_allocator,
            BufferCreateInfo { usage: BufferUsage::INDEX_BUFFER, ..Default::default() },
            AllocationCreateInfo { usage: MemoryUsage::Upload, ..Default::default() },
            indices.clone(),
        )
        .context("failed to create index buffer")?;

        Ok(Self { vertex_buffer, index_buffer, vertices, indices, material_index })
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

        self.vertex_buffer.write().expect("failed to write to vertex buffer").iter_mut().for_each(
            |vertex| {
                vertex.material_index += scene_material_arr_len;
            },
        );
    }
}
