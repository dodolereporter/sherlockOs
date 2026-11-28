#version 300 es
// Dual Kawase, halving: the pixel and its four diagonal neighbours, `half_texel` away.
precision mediump float;

uniform sampler2D tex;
uniform vec2 half_texel;
in vec2 uv;
out vec4 color;

void main() {
    vec2 d = half_texel;
    vec2 e = vec2(d.x, -d.y);
    vec4 sum = texture(tex, uv) * 4.0;
    sum += texture(tex, uv - d) + texture(tex, uv + d);
    sum += texture(tex, uv - e) + texture(tex, uv + e);
    color = sum / 8.0;
}
