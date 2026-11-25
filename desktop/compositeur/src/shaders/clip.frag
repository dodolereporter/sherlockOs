#version 100
// Contenu d'un client découpé en rectangle arrondi. Programme de texture de Smithay, qui
// remplace la ligne des définitions par NO_ALPHA, EXTERNAL et DEBUG_FLAGS selon le tampon.
// La découpe se lit sur `gl_FragCoord`, en pixels de l'image : elle vaut pour chaque surface
// de la fenêtre, quels que soient sa place, son échelle et son tampon.
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

// Rectangle de découpe dans l'image : x, y, largeur, hauteur.
uniform vec4 clip_rect;
uniform float radius;

float rounded_box(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
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
