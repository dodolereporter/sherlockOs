#version 300 es
// Dual Kawase, doubling: eight samples on a diamond around the pixel, `half_texel` apart.
precision mediump float;

uniform sampler2D tex;
uniform vec2 half_texel;
in vec2 uv;
out vec4 color;

void main() {
    vec2 d = half_texel;
    vec4 sum = texture(tex, uv + vec2(-2.0 * d.x, 0.0)) + texture(tex, uv + vec2(2.0 * d.x, 0.0));
    sum += texture(tex, uv + vec2(0.0, -2.0 * d.y)) + texture(tex, uv + vec2(0.0, 2.0 * d.y));
    sum += 2.0 * (texture(tex, uv + d) + texture(tex, uv - d));
    sum += 2.0 * (texture(tex, uv + vec2(d.x, -d.y)) + texture(tex, uv + vec2(-d.x, d.y)));
    color = sum / 12.0;
}
