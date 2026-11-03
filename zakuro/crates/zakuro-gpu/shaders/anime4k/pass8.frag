#version 450
// made from Anime4K_Upscale_CNN_x2_M.glsl, Anime4K by bloc97 (MIT, see that file),
// by anime4k.rs's write_screen_shaders test: do not edit.
layout(location = 0) out vec4 fragment;
layout(set = 0, binding = 0) uniform sampler2D MAIN;
layout(set = 0, binding = 1) uniform sampler2D conv2d_last_tf;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    int sub = (at.y % 2) * 2 + at.x % 2;
    vec4 base = texture(MAIN, gl_FragCoord.xy / vec2(textureSize(MAIN, 0) * 2));
    fragment = vec4(clamp(base.rgb + vec3(texelFetch(conv2d_last_tf, at / 2, 0)[sub], texelFetch(conv2d_last_tf, at / 2, 0)[sub], texelFetch(conv2d_last_tf, at / 2, 0)[sub]), 0.0, 1.0), 1.0);
}
