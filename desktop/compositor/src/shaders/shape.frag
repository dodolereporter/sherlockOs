// Rounded rectangle, filled and bordered: window borders, background discs, snap preview.
// Smithay pixel program: `v_coords` goes from 0 to 1 over the element, `size` is its size in
// pixels. Premultiplied colors.
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;

uniform vec4 fill;
uniform vec4 border;
uniform float radius;
uniform float border_width;

// Signed distance to the edge of a rounded rectangle centered on 0: negative inside.
float rounded_box(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    float d = rounded_box(v_coords * size - size * 0.5, size * 0.5, radius);
    // Coverage smoothed over one pixel, outside then inside the border.
    float outer = clamp(0.5 - d, 0.0, 1.0);
    float inner = clamp(0.5 - d - border_width, 0.0, 1.0);
    gl_FragColor = (fill * inner + border * (outer - inner)) * alpha;
}
