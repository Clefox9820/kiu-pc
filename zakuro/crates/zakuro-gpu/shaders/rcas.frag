#version 450
// AMD FidelityFX Super Resolution 1's robust contrast adaptive sharpening
// (RCAS), its 32-bit version, over the screens the FSR filter upscaled with
// EASU into an image the window's size, drawn pixel for pixel onto the
// window within one screen's place.
// compiled with glslc or naga:
//   glslc rcas.frag -o rcas.frag.spv
// the picture is a combined image sampler, taken as its image and its
// sampler apart; naga will not have two at one binding, so the sampler is
// written at binding 14 and moved onto 0 after, see
// examples/compile_present.rs.
//
//   Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
//   Permission is hereby granted, free of charge, to any person obtaining a
//   copy of this software and associated documentation files (the
//   "Software"), to deal in the Software without restriction, including
//   without limitation the rights to use, copy, modify, merge, publish,
//   distribute, sublicense, and/or sell copies of the Software, and to
//   permit persons to whom the Software is furnished to do so, subject to
//   the following conditions: The above copyright notice and this
//   permission notice shall be included in all copies or substantial
//   portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT
//   WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
//   THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//   NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
//   LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
//   OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
//   WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 colour;
layout(set = 0, binding = 0) uniform texture2D upscaled;
layout(set = 0, binding = 14) uniform sampler upscaled_sampler;

// the window pixels the screen covers, the first and the last, which the
// taps keep within so nothing around the screen darkens its border, then
// FsrRcasCon's sharpness, 2 to the minus its stops
layout(push_constant) uniform Sharpen {
    ivec4 rect;
    float sharpness;
} sharpen;

// how far a pixel may be pushed from its neighbours, as FSR has it
const float RCAS_LIMIT = 0.25 - 1.0 / 16.0;

vec3 tap(ivec2 at) {
    return texelFetch(sampler2D(upscaled, upscaled_sampler), clamp(at, sharpen.rect.xy, sharpen.rect.zw), 0).rgb;
}

void main() {
    //    b
    //  d e f
    //    h
    ivec2 p = ivec2(gl_FragCoord.xy);
    vec3 b = tap(p + ivec2(0, -1));
    vec3 d = tap(p + ivec2(-1, 0));
    vec3 e = tap(p);
    vec3 f = tap(p + ivec2(1, 0));
    vec3 h = tap(p + ivec2(0, 1));
    // the ring's least and most
    vec3 mn4 = min(min(min(b, d), f), h);
    vec3 mx4 = max(max(max(b, d), f), h);
    // the lobe that would take each channel to 0 or 1. a ring all black or
    // all white would divide 0 by 0, where the lobe is 0 anyway
    vec3 hit_min = min(mn4, e) / max(4.0 * mx4, vec3(1e-5));
    vec3 hit_max = (1.0 - max(mx4, e)) / min(4.0 * mn4 - 4.0, vec3(-1e-5));
    vec3 lobes = max(-hit_min, hit_max);
    float lobe = max(-RCAS_LIMIT, min(max(lobes.r, max(lobes.g, lobes.b)), 0.0)) * sharpen.sharpness;
    vec3 sharpened = (lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0);
    colour = vec4(sharpened, 1.0);
}
