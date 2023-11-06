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
