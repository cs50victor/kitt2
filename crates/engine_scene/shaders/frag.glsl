#version 450

#extension GL_EXT_nonuniform_qualifier : enable

struct GpuMaterial {
    uint base_color_texture_index;
    vec3 emissive_factor;
    uint emissive_texture_index;
    float ior;
    uint metallic_roughness_texture_index;
    uint normal_texture_index;
    uint occlusion_texture_index;
    uint specular_color_texture_index;
    uint specular_texture_index;
    uint specular_diffuse_texture_index;
    uint specular_glossiness_texture_index;
    uint transmission_texture_index;
    uint unlit;
    uint vol_thickness_texture_index;
    float vol_thickness_factor;
    float vol_attenuation_distance;
    vec3 vol_attenuation_color;
};

layout(set = 0, binding = 1) buffer MaterialsBufferObject {
    GpuMaterial materials[];
};

layout(set = 1, binding = 0) uniform sampler2D global_textures[];


layout(location = 0) in vec3 v_normal;
layout(location = 1) in vec2 v_tex_coords;
layout(location = 2) in flat uint mat_idx;

layout(location = 0) out vec4 out_color;


void main() {
    // Sample the material properties
    // GpuMaterial material = materials[mat_idx];
    out_color = texture(global_textures[nonuniformEXT(materials[mat_idx].base_color_texture_index)], v_tex_coords);
}
