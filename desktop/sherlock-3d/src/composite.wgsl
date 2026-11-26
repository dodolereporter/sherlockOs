// Lays the resolved scene over the target, inside a round (or rectangular) cut, over an
// optional background. Premultiplied alpha, like iced.

struct Params {
    // Top left corner of the area in the target, in pixels.
    origin: vec2<f32>,
    // Center of the cut, relative to the origin, in pixels.
    center: vec2<f32>,
    // Radius of the cut in pixels; negative for no cut.
    radius: f32,
    // 1 when the target stores sRGB values without an sRGB format (iced with web colors).
    encode_srgb: f32,
    pad0: f32,
    pad1: f32,
    // Linear, premultiplied.
    background: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var scene: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the viewport.
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn encode(linear: f32) -> f32 {
    if linear <= 0.0031308 {
        return linear * 12.92;
    }
    return 1.055 * pow(linear, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let local = position.xy - params.origin;
    let size = vec2<f32>(textureDimensions(scene));
    let texel = vec2<i32>(clamp(floor(local), vec2<f32>(0.0), size - 1.0));
    let sherlock = textureLoad(scene, texel, 0);
    var color = sherlock + params.background * (1.0 - sherlock.a);
    if params.radius >= 0.0 {
        // One pixel of antialiasing on the edge of the bubble.
        let coverage = clamp(params.radius - distance(local, params.center) + 0.5, 0.0, 1.0);
        color = color * coverage;
    }
    if params.encode_srgb > 0.5 && color.a > 0.0 {
        let straight = color.rgb / color.a;
        color = vec4<f32>(
            vec3<f32>(encode(straight.r), encode(straight.g), encode(straight.b)) * color.a,
            color.a,
        );
    }
    return color;
}
