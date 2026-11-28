#version 100
// Client content clipped to a rounded rectangle. Smithay texture program, which replaces the
// defines line with NO_ALPHA, EXTERNAL and DEBUG_FLAGS depending on the buffer.
// The clip is read from `gl_FragCoord`, in frame pixels: it holds for every surface of the
// window, whatever its place, its scale and its buffer.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
// The footprint of a pixel in the texture, to shrink a window smoothly (lot D-33c). Without the
// extension, one sample, as before.
#ifdef GL_OES_standard_derivatives
#extension GL_OES_standard_derivatives : enable
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

// Clip rectangle in the frame: x, y, width, height.
uniform vec4 clip_rect;
uniform float radius;
// Texels of the buffer per pixel drawn: past 1.5, the window is shrunk (Mission Control, its
// thumbnails) and one sample would alias.
uniform float minify;

vec4 sample_at(vec2 coords) {
    vec4 color = texture2D(tex, coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    return color;
}

float rounded_box(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec4 color;
#ifdef GL_OES_standard_derivatives
    if (minify > 1.5) {
        // 4 x 4 samples over the footprint of the pixel: a box filter, close to what a mipmap
        // would give, for a window shrunk up to some 16 times.
        vec2 dx = dFdx(v_coords) * 0.25;
        vec2 dy = dFdy(v_coords) * 0.25;
        color = vec4(0.0);
        for (int i = 0; i < 4; i++) {
            for (int j = 0; j < 4; j++) {
                color += sample_at(v_coords + (float(i) - 1.5) * dx + (float(j) - 1.5) * dy);
            }
        }
        color *= 0.0625;
    } else {
        color = sample_at(v_coords);
    }
#else
    color = sample_at(v_coords);
#endif
    vec2 half_size = clip_rect.zw * 0.5;
    float d = rounded_box(gl_FragCoord.xy - clip_rect.xy - half_size, half_size, radius);
    color *= alpha * clamp(0.5 - d, 0.0, 1.0);
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
