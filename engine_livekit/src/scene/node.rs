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
                    scene.all_textures.insert(curr_scene_textures_len, texture.to_owned());
                    curr_scene_textures_len += 1;
                });

                model.meshes.iter_mut().for_each(|mesh| {
                    mesh.update_material_index(prev_scene_textures_len);
                });

                model.materials.iter_mut().for_each(|material| {
                    material.update_texture_indexs(prev_scene_textures_len);
                    scene.all_materials.insert(curr_scene_materials_len, material.to_owned());
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
