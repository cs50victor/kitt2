#version 450
// vertex shader maps the framebuffer cordinates to clip coordinates | 
// i.e [0,0] by [1920,1080] -> [-1,-1] by [1,1]

layout(location = 0) in vec3 position;
layout(location = 1) in vec2 tex_coords;
layout(location = 2) in vec3 normal;
layout(location = 5) in uint material_index;

layout(location = 0) out vec3 v_normal;
layout(location = 1) out vec2 v_tex_coords;
layout(location = 2) out flat uint mat_idx;


// model-view-projection matrix
layout(set = 0, binding = 0) uniform UniformBufferObject {
    mat4 view;
    mat4 proj;
} ubo;

layout(push_constant) uniform PushConstants {
    mat4 model;
} pcs;

void main() {
    mat4 modelview = ubo.view * pcs.model; 
    v_normal = transpose(inverse(mat3(modelview))) * normal;
    v_tex_coords = tex_coords;
    mat_idx = material_index;
    gl_Position = ubo.proj * modelview * vec4(position, 1.0);
}
