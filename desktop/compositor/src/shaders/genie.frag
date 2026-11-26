#version 100
// Genie of a window minimized into its dock icon, or coming back from it (lot D-19). Smithay
// texture program: the texture is the window drawn as at rest, the element covers the area of
// the deformed window. Each pixel finds the point of the window it shows, the exact inverse of
// the deformation of `Genie::row` in `src/genie.rs`, which it must follow.
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

// Window and icon in frame pixels (x, y, width, height), mirrored vertically when the icon is
// above the window (`mirror` is -1 then): the window always goes down.
uniform vec4 window_rect;
uniform vec4 icon_rect;
// Area of the element in frame pixels, which `v_coords` spans.
uniform vec4 area;
uniform float progress;
uniform float mirror;

// `BEND` in `src/genie.rs`.
const float BEND = 0.4;

float ease(float x) {
    x = clamp(x, 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x);
}

void main() {
    vec2 p = area.xy + v_coords * area.zw;
    p.y *= mirror;
    float slide = ease((progress - BEND) / (1.0 - BEND));
    float top = mix(window_rect.y, icon_rect.y, slide);
    float bottom = mix(window_rect.y + window_rect.w, icon_rect.y + icon_rect.w, slide);
    float bend = ease(progress / BEND);
    float reach = ease((p.y - window_rect.y) / max(icon_rect.y - window_rect.y, 1.0));
    float f = bend * reach;
    float left = mix(window_rect.x, icon_rect.x, f);
    float right = mix(window_rect.x + window_rect.z, icon_rect.x + icon_rect.z, f);
    vec2 uv = vec2((p.x - left) / max(right - left, 0.001), (p.y - top) / max(bottom - top, 0.001));
    if (mirror < 0.0)
        uv.y = 1.0 - uv.y;
    // Smoothed over one pixel along each edge, measured on the screen.
    float cover = clamp(min(p.x - left, right - p.x) + 0.5, 0.0, 1.0)
        * clamp(min(p.y - top, bottom - p.y) + 0.5, 0.0, 1.0);
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
