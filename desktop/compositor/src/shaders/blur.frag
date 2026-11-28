#version 100
// The blurred picture behind a glass surface (lot D-37), drawn over its region with a little
// more saturation. Smithay texture program: the defines line becomes NO_ALPHA, EXTERNAL and
// DEBUG_FLAGS depending on the case. The picture is read at `gl_FragCoord`, in the frame's own
// pixels, as it was copied from the frame: it holds whatever the frame transform.
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

// Where the picture was copied from, in the frame: x, y, width, height.
uniform vec4 area;
uniform float saturation;

void main() {
    vec2 uv = (gl_FragCoord.xy - area.xy) / area.zw;
    vec3 color = texture2D(tex, uv).rgb;
    float luma = dot(color, vec3(0.2126, 0.7152, 0.0722));
    color = clamp(mix(vec3(luma), color, saturation), 0.0, 1.0);
    gl_FragColor = vec4(color, 1.0) * alpha;
}
