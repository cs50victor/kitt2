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
