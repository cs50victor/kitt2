use glam::Mat4;

use crate::{materials::MeshMaterial, mesh::Mesh, texture::Texture};

#[derive(Clone, Debug)]
pub struct Model {
    pub meshes: Vec<Mesh>,
    pub textures: Vec<Texture>,
    pub transforms: Vec<Mat4>,
    pub materials: Vec<MeshMaterial>,
}
