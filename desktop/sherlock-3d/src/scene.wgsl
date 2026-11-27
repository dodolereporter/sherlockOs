// The mascot, lit by fixed lights tied to the camera, like the key, fill and back lights of the
// model's reference renders (Blender, `sherlock_views.py`): a large soft key from the top left,
// a soft fill from the right, a thin back light that lifts the outline off a dark background.
// Nothing moves and no shadow is computed: a few products per pixel. The occlusion of the
// hollows, baked in the vertex colors, multiplies the result; a slight darkening toward the
// bottom of the body adds weight. Written premultiplied. Skinning with 4 bones per vertex.

struct Globals {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    joints: array<mat4x4<f32>, 32>,
};

struct Material {
    base: vec4<f32>,
    // Linear emission, then roughness (0 shiny, 1 matt).
    emissive_roughness: vec4<f32>,
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
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // Height in the space of the model, the feet at 0.
    @location(2) height: f32,
    // Normal in the space of the camera.
    @location(3) normal: vec3<f32>,
};

// The bottom of the body is darkened by up to this share, fading out from LOW to HIGH (heights
// in the space of the model: the scarf is at about 1.8, the top of the cap at about 3).
const BOTTOM_SHADE: f32 = 0.18;
const LOW: f32 = 0.9;
const HIGH: f32 = 1.9;

// Directions toward the lights, in the space of the camera (x right, y up, z toward the
// viewer), from the light positions of the reference renders around the mascot.
const KEY: vec3<f32> = vec3<f32>(-0.560, 0.517, 0.647);
const FILL: vec3<f32> = vec3<f32>(0.705, 0.059, 0.705);
const BACK: vec3<f32> = vec3<f32>(0.384, 0.512, -0.768);
// Strengths: the light a white surface facing each one receives.
const AMBIENT: f32 = 0.40;
const KEY_STRENGTH: f32 = 0.56;
const FILL_STRENGTH: f32 = 0.14;
const BACK_STRENGTH: f32 = 0.35;
// The key is a large soft light: its falloff wraps past the terminator (a "wrap" Lambert).
const KEY_WRAP: f32 = 0.45;
// The highlight of the shiniest material (roughness 0); a matt cap keeps a faint sheen.
const SPECULAR: f32 = 0.26;
const MATT_SHEEN: f32 = 0.02;

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let skin = globals.joints[v.joints.x] * v.weights.x
        + globals.joints[v.joints.y] * v.weights.y
        + globals.joints[v.joints.z] * v.weights.z
        + globals.joints[v.joints.w] * v.weights.w;
    var out: VertexOut;
    let world = skin * vec4<f32>(v.position, 1.0);
    out.clip = globals.view_proj * world;
    out.height = world.y;
    out.uv = v.uv;
    out.color = v.color;
    out.normal = (globals.view * skin * vec4<f32>(v.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let texel = textureSample(base_texture, base_sampler, in.uv);
    // Vertex color (the baked occlusion) x base color x texture.
    let albedo = in.color * material.base * texel;
    if albedo.a < material.alpha_cutoff {
        discard;
    }
    // Double-sided materials: the back of a face turns its normal around.
    var n = normalize(in.normal);
    if !front {
        n = -n;
    }
    let roughness = material.emissive_roughness.w;

    let key = max((dot(n, KEY) + KEY_WRAP) / (1.0 + KEY_WRAP), 0.0);
    let fill = max(dot(n, FILL), 0.0);
    // The back light only shows on the edges that turn away from the viewer.
    let edge = 1.0 - max(n.z, 0.0);
    let back = max(dot(n, BACK), 0.0) * edge * edge;
    let diffuse = AMBIENT + KEY_STRENGTH * key + FILL_STRENGTH * fill + BACK_STRENGTH * back;

    // A soft highlight of the key, wider and fainter as the material gets matt.
    let half_way = normalize(KEY + vec3<f32>(0.0, 0.0, 1.0));
    let shininess = mix(96.0, 6.0, roughness);
    let gloss = max((1.0 - roughness) * (1.0 - roughness), MATT_SHEEN);
    let highlight = SPECULAR * gloss * pow(max(dot(n, half_way), 0.0), shininess);

    let shade = 1.0 - BOTTOM_SHADE * (1.0 - smoothstep(LOW, HIGH, in.height));
    let rgb = (albedo.rgb * diffuse + highlight * in.color.rgb) * shade
        + material.emissive_roughness.rgb;
    return vec4<f32>(rgb * albedo.a, albedo.a);
}
