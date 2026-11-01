#version 450
// SMAA 1x's first pass, luma edge detection, over one screen at its own
// size before the FSR filter upscales it. reads the screen where it lies in
// its image, its own or the renderer's, and writes the edges at its west
// (red) and north (green) to an R8G8 target cleared to no edge.
// compiled with glslc or naga:
//   glslc smaa_edges.frag -o smaa_edges.frag.spv
// the picture is a combined image sampler, taken as its image and its
// sampler apart; naga will not have two at one binding, so the sampler is
// written at binding 14 and moved onto 0 after, see
// examples/compile_present.rs.
//
// SMAA: Enhanced Subpixel Morphological Antialiasing,
// https://github.com/iryoku/smaa
//   Copyright (C) 2013 Jorge Jimenez (jorge@iryoku.com)
//   Copyright (C) 2013 Jose I. Echevarria (joseignacioechevarria@gmail.com)
//   Copyright (C) 2013 Belen Masia (bmasia@unizar.es)
//   Copyright (C) 2013 Fernando Navarro (fernandn@microsoft.com)
//   Copyright (C) 2013 Diego Gutierrez (diegog@unizar.es)
//   Permission is hereby granted, free of charge, to any person obtaining a
//   copy this software and associated documentation files (the "Software"),
//   to deal in the Software without restriction, including without
//   limitation the rights to use, copy, modify, merge, publish, distribute,
//   sublicense, and/or sell copies of the Software, and to permit persons to
//   whom the Software is furnished to do so, subject to the following
//   conditions: The above copyright notice and this permission notice shall
//   be included in all copies or substantial portions of the Software. As
//   clarification, there is no requirement that the copyright notice and
//   permission be included in binary distributions of the Software.
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//   OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//   MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//   NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//   DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//   OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//   USE OR OTHER DEALINGS IN THE SOFTWARE.
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 colour;
layout(set = 0, binding = 0) uniform texture2D screen;
layout(set = 0, binding = 14) uniform sampler screen_sampler;

// where the screen lies in its image, its corner and size as texture
// coordinates, then the corners inset by half a texel, which samples keep
// within, then SMAA's metrics of the screen: 1 / width, 1 / height, width
// and height
layout(push_constant) uniform Pass {
    vec4 area;
    vec4 bounds;
    vec4 metrics;
} pass;

// SMAA's HIGH preset
const float THRESHOLD = 0.1;
const float LOCAL_CONTRAST_ADAPTATION_FACTOR = 2.0;

// the luma of the screen at t, a texture coordinate over the screen alone
float luma(vec2 t) {
    vec2 at = clamp(pass.area.xy + t * pass.area.zw, pass.bounds.xy, pass.bounds.zw);
    return dot(textureLod(sampler2D(screen, screen_sampler), at, 0.0).rgb, vec3(0.2126, 0.7152, 0.0722));
}

void main() {
    vec2 texel = pass.metrics.xy;
    float L = luma(uv);
    float Lleft = luma(uv + vec2(-1.0, 0.0) * texel);
    float Ltop = luma(uv + vec2(0.0, -1.0) * texel);

    vec2 delta_left_top = abs(L - vec2(Lleft, Ltop));
    vec2 edges = step(vec2(THRESHOLD), delta_left_top);
    if (dot(edges, vec2(1.0)) == 0.0) {
        discard;
    }

    float Lright = luma(uv + vec2(1.0, 0.0) * texel);
    float Lbottom = luma(uv + vec2(0.0, 1.0) * texel);
    vec2 max_delta = max(delta_left_top, abs(L - vec2(Lright, Lbottom)));

    float Lleftleft = luma(uv + vec2(-2.0, 0.0) * texel);
    float Ltoptop = luma(uv + vec2(0.0, -2.0) * texel);
    max_delta = max(max_delta, abs(vec2(Lleft, Ltop) - vec2(Lleftleft, Ltoptop)));
    float final_delta = max(max_delta.x, max_delta.y);

    // local contrast adaptation
    edges *= step(vec2(final_delta), LOCAL_CONTRAST_ADAPTATION_FACTOR * delta_left_top);
    colour = vec4(edges, 0.0, 0.0);
}
