// Rectangle arrondi, rempli et bordé : bordure des fenêtres, disques du fond, aperçu du calage.
// Programme de pixels de Smithay : `v_coords` va de 0 à 1 sur l'élément, `size` est sa taille
// en pixels. Couleurs prémultipliées.
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

// Distance signée au bord d'un rectangle arrondi centré en 0 : négative à l'intérieur.
float rounded_box(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    float d = rounded_box(v_coords * size - size * 0.5, size * 0.5, radius);
    // Couverture lissée sur un pixel, dehors puis à l'intérieur de la bordure.
    float outer = clamp(0.5 - d, 0.0, 1.0);
    float inner = clamp(0.5 - d - border_width, 0.0, 1.0);
    gl_FragColor = (fill * inner + border * (outer - inner)) * alpha;
}
