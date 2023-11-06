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
