#version 100
// Genie of a window minimized into its dock icon, or coming back from it (lots D-19, D-19b).
// Smithay texture program: the texture is the window drawn as at rest, the element covers the
// area of the deformed window. Each pixel finds the point of the window it shows, the exact
// inverse of the deformation of `Genie::point` in `src/genie.rs`, which it must follow.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// The window at rest in frame pixels (x, y, width, height).
uniform vec4 window_rect;
// Window and icon seen along the way to the dock: their far and near ends along it, then their
// two ends across (`Span` in `src/genie.rs`).
uniform vec4 window_span;
uniform vec4 icon_span;
// Start and end of the funnel along, how far its sides are bent, and the clip: nothing past it
// shows, it is behind the dock's glass.
uniform vec4 funnel;
// Area of the element in frame pixels, which `v_coords` spans.
uniform vec4 area;
// 1 when the dock is on the left or right edge, then -1 when the way to it goes toward lower
// coordinates (dock at the top or on the left), 1 otherwise.
uniform vec2 axis;
uniform float progress;

// `DELAY` in `src/genie.rs`.
const float DELAY = 0.4;
const float PI = 3.14159265;

// A point of the frame as (along, across), along growing toward the dock: `Side::orient`.
vec2 orient(vec2 p) {
    vec2 q = axis.x > 0.5 ? p : p.yx;
    return vec2(q.x * axis.y, q.y);
}

// `Side::unorient`.
vec2 unorient(vec2 q) {
    vec2 p = vec2(q.x * axis.y, q.y);
    return axis.x > 0.5 ? p : p.yx;
}

// Where row `r` of the window is along, 0 its far edge, 1 its near one: `Shape::along`.
float along_of(float r) {
    float rest = mix(window_span.x, window_span.y, r);
    float end = mix(icon_span.x, icon_span.y, r);
    float t = clamp((progress - (1.0 - r) * DELAY) / (1.0 - DELAY), 0.0, 1.0);
    return mix(rest, end, t * t);
}

void main() {
    vec2 q = orient(area.xy + v_coords * area.zw);
    float far = along_of(0.0);
    float near = min(along_of(1.0), funnel.w);
    // The row drawn here: `along_of` grows with the row, found by halving.
    float low = 0.0;
    float high = 1.0;
    for (int i = 0; i < 20; i++) {
        float middle = 0.5 * (low + high);
        if (along_of(middle) < q.x)
            low = middle;
        else
            high = middle;
    }
    float row = 0.5 * (low + high);
    // The sides: `Shape::across`.
    float x = clamp((q.x - funnel.x) / (funnel.y - funnel.x), 0.0, 1.0);
    float f = funnel.z * (1.0 - cos(PI * x)) * 0.5;
    float left = mix(window_span.z, icon_span.z, f);
    float right = mix(window_span.w, icon_span.w, f);
    float column = (q.y - left) / max(right - left, 0.001);
    vec2 rest = vec2(mix(window_span.x, window_span.y, row), mix(window_span.z, window_span.w, column));
    vec2 uv = (unorient(rest) - window_rect.xy) / window_rect.zw;
    // Smoothed over one pixel along each edge, measured on the screen.
    float cover = clamp(min(q.x - far, near - q.x) + 0.5, 0.0, 1.0)
        * clamp(min(q.y - left, right - q.y) + 0.5, 0.0, 1.0);
    vec4 color = texture2D(tex, clamp(uv, 0.0, 1.0));
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    color *= alpha * cover;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
