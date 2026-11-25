// Drop shadow of a window, like `box-shadow` in CSS: a rounded rectangle blurred by a Gaussian,
// cleared under the window itself. Exact integral in x, four samples in y, after Evan Wallace,
// "Fast Rounded Rectangle Shadows" (2020, public domain).
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;

uniform vec4 color;
// Rectangle that casts the shadow, then window, in the element: x, y, width, height.
uniform vec4 caster;
uniform vec4 window;
uniform float radius;
uniform float sigma;

float gaussian(float x) {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066283 * sigma);
}

vec2 erf(vec2 x) {
    vec2 s = sign(x);
    vec2 a = abs(x);
    x = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    x *= x;
    return s - s / (x * x);
}

float shadow_x(float x, float y, vec2 half_size) {
    float delta = min(half_size.y - radius - abs(y), 0.0);
    float curved = half_size.x - radius + sqrt(max(0.0, radius * radius - delta * delta));
    vec2 integral = 0.5 + 0.5 * erf((x + vec2(-curved, curved)) * (0.70710678 / sigma));
    return integral.y - integral.x;
}

float shadow(vec2 p, vec2 half_size) {
    float start = clamp(-3.0 * sigma, p.y - half_size.y, p.y + half_size.y);
    float end = clamp(3.0 * sigma, p.y - half_size.y, p.y + half_size.y);
    float dy = (end - start) / 4.0;
    float y = start + dy * 0.5;
    float value = 0.0;
    for (int i = 0; i < 4; i++) {
        value += shadow_x(p.x, p.y - y, half_size) * gaussian(y) * dy;
        y += dy;
    }
    return value;
}

float rounded_box(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 p = v_coords * size;
    float d = rounded_box(p - window.xy - window.zw * 0.5, window.zw * 0.5, radius);
    // Under the window, nothing to compute: the largest part of the element.
    if (d < -1.0) {
        gl_FragColor = vec4(0.0);
        return;
    }
    float value = shadow(p - caster.xy - caster.zw * 0.5, caster.zw * 0.5);
    gl_FragColor = color * (value * clamp(0.5 + d, 0.0, 1.0) * alpha);
}
