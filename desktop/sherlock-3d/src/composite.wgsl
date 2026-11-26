// Lays the resolved scene over the target, inside a round (or rectangular) cut, over a soft
// drop shadow of the mascot and an optional background. Premultiplied alpha, like iced.

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
    // Where the shadow falls from the mascot, its blur radius, in pixels, and its opacity.
    shadow_offset: vec2<f32>,
    shadow_blur: f32,
    shadow_opacity: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var scene_sampler: sampler;

// Taps of the shadow's blur, spread on a disc along the golden angle.
const TAPS: u32 = 24u;

// How much of the mascot's silhouette, blurred, covers `local`.
fn shadow(local: vec2<f32>, size: vec2<f32>) -> f32 {
    let origin = local - params.shadow_offset;
    var sum = 0.0;
    for (var i = 0u; i < TAPS; i = i + 1u) {
        let r = sqrt((f32(i) + 0.5) / f32(TAPS)) * params.shadow_blur;
        let a = f32(i) * 2.3999632;
        let at = (origin + vec2<f32>(cos(a), sin(a)) * r) / size;
        if all(at >= vec2<f32>(0.0)) && all(at <= vec2<f32>(1.0)) {
            sum = sum + textureSampleLevel(scene, scene_sampler, at, 0.0).a;
        }
    }
    return sum / f32(TAPS) * params.shadow_opacity;
}

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
    let dark = shadow(local, size);
    let ground = vec4<f32>(0.0, 0.0, 0.0, dark) + params.background * (1.0 - dark);
    var color = sherlock + ground * (1.0 - sherlock.a);
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
