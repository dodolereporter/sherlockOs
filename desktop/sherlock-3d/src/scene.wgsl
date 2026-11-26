// The mascot, unlit: its soft shadows are baked in the vertex colors. Color = vertex color x
// base color x texture, written premultiplied. Skinning with 4 bones per vertex.

struct Globals {
    view_proj: mat4x4<f32>,
    joints: array<mat4x4<f32>, 32>,
};

struct Material {
    base: vec4<f32>,
    // Fragments whose alpha is under it are dropped (0 for opaque and blended materials).
    alpha_cutoff: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let skin = globals.joints[v.joints.x] * v.weights.x
        + globals.joints[v.joints.y] * v.weights.y
        + globals.joints[v.joints.z] * v.weights.z
        + globals.joints[v.joints.w] * v.weights.w;
    var out: VertexOut;
    out.clip = globals.view_proj * skin * vec4<f32>(v.position, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(base_texture, base_sampler, in.uv);
    let color = in.color * material.base * texel;
    if color.a < material.alpha_cutoff {
        discard;
    }
    return vec4<f32>(color.rgb * color.a, color.a);
}
