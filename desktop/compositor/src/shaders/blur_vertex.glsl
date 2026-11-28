#version 300 es
// One triangle that covers the whole target, without vertex buffer: the passes of the blur
// behind glass (lot D-37) read their source at `uv`.
out vec2 uv;

void main() {
    uv = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}
