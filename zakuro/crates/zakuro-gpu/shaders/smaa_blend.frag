#version 450
// SMAA 1x's third pass, neighborhood blending: mixes each pixel of one
// screen, at its own size, with the neighbour across an edge by the weights
// the second pass found, writing the antialiased screen the FSR filter then
// upscales. reads the screen where it lies in its image, as the first pass
// does.
// compiled with glslc or naga:
//   glslc smaa_blend.frag -o smaa_blend.frag.spv
// each picture is a combined image sampler, taken as its image and its
// sampler apart; naga will not have two at one binding, so each sampler is
// written at binding 14 of its set and moved onto 0 after, see
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
// the screen, in set 0 as the present shader has it, and the weights
layout(set = 0, binding = 0) uniform texture2D screen;
layout(set = 0, binding = 14) uniform sampler screen_sampler;
layout(set = 1, binding = 0) uniform texture2D weights_image;
layout(set = 1, binding = 14) uniform sampler weights_sampler;

// where the screen lies in its image, its corner and size as texture
// coordinates, then the corners inset by half a texel, which samples keep
// within, then SMAA's metrics of the screen: 1 / width, 1 / height, width
// and height
layout(push_constant) uniform Pass {
    vec4 area;
    vec4 bounds;
    vec4 metrics;
} pass;

// the screen at t, a texture coordinate over the screen alone, bilinear
vec3 screen_at(vec2 t) {
    vec2 at = clamp(pass.area.xy + t * pass.area.zw, pass.bounds.xy, pass.bounds.zw);
    return textureLod(sampler2D(screen, screen_sampler), at, 0.0).rgb;
}

vec4 weights_at(vec2 t) {
    return textureLod(sampler2D(weights_image, weights_sampler), t, 0.0);
}

void main() {
    vec4 m = pass.metrics;
    vec4 offset = m.xyxy * vec4(1.0, 0.0, 0.0, 1.0) + uv.xyxy;

    // the weights of this pixel: right, top, left, bottom
    vec4 here = weights_at(uv);
    vec4 a = vec4(weights_at(offset.xy).w, weights_at(offset.zw).y, here.z, here.x);

    if (dot(a, vec4(1.0)) < 1e-5) {
        colour = vec4(screen_at(uv), 1.0);
        return;
    }

    // the stronger of horizontal and vertical
    bool horizontal = max(a.x, a.z) > max(a.y, a.w);
    vec4 blending_offset = vec4(0.0, a.y, 0.0, a.w);
    vec2 blending_weight = a.yw;
    if (horizontal) {
        blending_offset = vec4(a.x, 0.0, a.z, 0.0);
        blending_weight = a.xz;
    }
    blending_weight /= dot(blending_weight, vec2(1.0));

    // bilinear filtering mixes the pixel with the neighbour chosen
    vec4 blending_coord = blending_offset * vec4(m.xy, -m.xy) + uv.xyxy;
    vec3 blended = blending_weight.x * screen_at(blending_coord.xy) + blending_weight.y * screen_at(blending_coord.zw);
    colour = vec4(blended, 1.0);
}
