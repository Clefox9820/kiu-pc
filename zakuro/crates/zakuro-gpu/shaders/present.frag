#version 450
// compiled twice, as it is and with STEREO defined, which draws the top
// screen's two pictures in one for 3D, with glslc or naga:
//   glslc present.frag -o present.frag.spv
//   glslc -DSTEREO present.frag -o present_stereo.frag.spv
// each picture is a combined image sampler, which a shader may take as its
// image and its sampler apart, at the same binding. naga reads only that,
// and will not have two at one binding, so its samplers are written at
// 14 and 15 and moved onto their images' 0 and 1 after, see
// examples/compile_present.rs.
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 colour;
layout(set = 0, binding = 0) uniform texture2D screen;
layout(set = 0, binding = 14) uniform sampler screen_sampler;
#ifdef STEREO
layout(set = 0, binding = 1) uniform texture2D right_eye;
layout(set = 0, binding = 15) uniform sampler right_eye_sampler;
#endif

// the picture of eye 0, the screen, or of eye 1, the right eye's: sampled,
// one texel, and its size
vec4 sampled(int eye, vec2 at) {
#ifdef STEREO
    if (eye == 1) {
        return texture(sampler2D(right_eye, right_eye_sampler), at);
    }
#endif
    return texture(sampler2D(screen, screen_sampler), at);
}

vec4 texel_at(int eye, ivec2 at) {
#ifdef STEREO
    if (eye == 1) {
        return texelFetch(sampler2D(right_eye, right_eye_sampler), at, 0);
    }
#endif
    return texelFetch(sampler2D(screen, screen_sampler), at, 0);
}

vec2 size_of(int eye) {
#ifdef STEREO
    if (eye == 1) {
        return vec2(textureSize(sampler2D(right_eye, right_eye_sampler), 0));
    }
#endif
    return vec2(textureSize(sampler2D(screen, screen_sampler), 0));
}
// where the screen lies in the image, its corner and size as texture
// coordinates, then the corners inset by half a texel, which samples keep
// within so filtering takes nothing from around it. the whole image when it
// holds just the screen. then how a screen drawn bigger than its pixels is
// filtered: 0 smooth, 1 pixels, 2 sharp, 3 AMD FSR 1's EASU.
layout(push_constant) uniform Crop {
    vec4 area;
    vec4 bounds;
    int mode;
#ifdef STEREO
    // how the eyes are shown, 1 in red and cyan, 2 on alternate rows,
    // whether they swap, then where the right eye's picture lies
    int stereo;
    int swap;
    int unused;
    vec4 right_area;
    vec4 right_bounds;
#endif
} crop;

// AMD FidelityFX Super Resolution 1's edge adaptive upscaling (EASU), its
// 32-bit version taking single texels rather than gathers:
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

// one tap of the kernel: a lanczos 2 shape stretched along the edge
void easu_tap(inout vec3 sum, inout float weights, vec2 off, vec2 dir, vec2 len, float lob, float clp, vec3 c) {
    vec2 v = vec2(off.x * dir.x + off.y * dir.y, off.x * -dir.y + off.y * dir.x) * len;
    float d2 = min(dot(v, v), clp);
    float wb = 2.0 / 5.0 * d2 - 1.0;
    float wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    float w = (25.0 / 16.0 * wb - (25.0 / 16.0 - 1.0)) * wa;
    sum += c * w;
    weights += w;
}

// the direction and length of the edge around one of the 4 nearest texels,
// weighed by how near it is
void easu_set(inout vec2 dir, inout float len, float w, float la, float lb, float lc, float ld, float le) {
    float dc = ld - lc;
    float cb = lc - lb;
    float lenx = 1.0 / max(max(abs(dc), abs(cb)), 1e-8);
    float dirx = ld - lb;
    dir.x += dirx * w;
    lenx = clamp(abs(dirx) * lenx, 0.0, 1.0);
    len += lenx * lenx * w;
    float ec = le - lc;
    float ca = lc - la;
    float leny = 1.0 / max(max(abs(ec), abs(ca)), 1e-8);
    float diry = le - la;
    dir.y += diry * w;
    leny = clamp(abs(diry) * leny, 0.0, 1.0);
    len += leny * leny * w;
}

// the screen at texel position p, upscaled; lo and hi the texels it holds
vec3 easu(int eye, vec2 p, ivec2 lo, ivec2 hi) {
    vec2 fp = floor(p);
    vec2 pp = p - fp;
    ivec2 f = ivec2(fp);
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    vec3 b = texel_at(eye, clamp(f + ivec2(0, -1), lo, hi)).rgb;
    vec3 c = texel_at(eye, clamp(f + ivec2(1, -1), lo, hi)).rgb;
    vec3 e = texel_at(eye, clamp(f + ivec2(-1, 0), lo, hi)).rgb;
    vec3 F = texel_at(eye, clamp(f, lo, hi)).rgb;
    vec3 g = texel_at(eye, clamp(f + ivec2(1, 0), lo, hi)).rgb;
    vec3 h = texel_at(eye, clamp(f + ivec2(2, 0), lo, hi)).rgb;
    vec3 i = texel_at(eye, clamp(f + ivec2(-1, 1), lo, hi)).rgb;
    vec3 j = texel_at(eye, clamp(f + ivec2(0, 1), lo, hi)).rgb;
    vec3 k = texel_at(eye, clamp(f + ivec2(1, 1), lo, hi)).rgb;
    vec3 l = texel_at(eye, clamp(f + ivec2(2, 1), lo, hi)).rgb;
    vec3 n = texel_at(eye, clamp(f + ivec2(0, 2), lo, hi)).rgb;
    vec3 o = texel_at(eye, clamp(f + ivec2(1, 2), lo, hi)).rgb;
    // luma times 2
    vec3 weigh = vec3(0.5, 1.0, 0.5);
    float bL = dot(b, weigh), cL = dot(c, weigh), eL = dot(e, weigh), fL = dot(F, weigh);
    float gL = dot(g, weigh), hL = dot(h, weigh), iL = dot(i, weigh), jL = dot(j, weigh);
    float kL = dot(k, weigh), lL = dot(l, weigh), nL = dot(n, weigh), oL = dot(o, weigh);
    vec2 dir = vec2(0.0);
    float len = 0.0;
    easu_set(dir, len, (1.0 - pp.x) * (1.0 - pp.y), bL, eL, fL, gL, jL);
    easu_set(dir, len, pp.x * (1.0 - pp.y), cL, fL, gL, hL, kL);
    easu_set(dir, len, (1.0 - pp.x) * pp.y, fL, iL, jL, kL, nL);
    easu_set(dir, len, pp.x * pp.y, gL, jL, kL, lL, oL);
    float dirr = dot(dir, dir);
    bool zero = dirr < 1.0 / 32768.0;
    dirr = zero ? 1.0 : inversesqrt(dirr);
    dir.x = zero ? 1.0 : dir.x;
    dir *= dirr;
    len = len * 0.5;
    len *= len;
    float stretch = dot(dir, dir) / max(max(abs(dir.x), abs(dir.y)), 1e-8);
    vec2 len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    float lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    float clp = 1.0 / lob;
    vec3 min4 = min(min(F, g), min(j, k));
    vec3 max4 = max(max(F, g), max(j, k));
    vec3 sum = vec3(0.0);
    float weights = 0.0;
    easu_tap(sum, weights, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, b);
    easu_tap(sum, weights, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, c);
    easu_tap(sum, weights, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, i);
    easu_tap(sum, weights, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, j);
    easu_tap(sum, weights, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, F);
    easu_tap(sum, weights, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, e);
    easu_tap(sum, weights, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, k);
    easu_tap(sum, weights, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, l);
    easu_tap(sum, weights, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, h);
    easu_tap(sum, weights, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, g);
    easu_tap(sum, weights, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, o);
    easu_tap(sum, weights, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, n);
    // normalized, and kept within the 4 nearest so it does not ring
    return min(max4, max(min4, sum / weights));
}

// a screen drawn bigger than the window shows it is averaged over every
// texel a window pixel covers, which smooths its edges the way drawing at a
// higher resolution should. one drawn smaller is filtered as asked. a
// little over a texel a pixel still counts as one, at 1:1 the derivatives
// land on either side of it.
vec3 shown(int eye, vec4 area, vec4 bounds) {
    vec2 at = area.xy + uv * area.zw;
    vec2 size = size_of(eye);
    vec2 covered = fwidth(at) * size;
    if (max(covered.x, covered.y) <= 1.001) {
        vec2 texel = at * size;
        if (crop.mode == 3) {
            // the texels the screen holds, from the inset corners
            ivec2 lo = ivec2(floor(bounds.xy * size));
            ivec2 hi = ivec2(floor(bounds.zw * size));
            return easu(eye, texel - 0.5, lo, hi);
        }
        if (crop.mode == 1) {
            // the nearest texel, its middle sampled
            at = (floor(texel) + 0.5) / size;
        } else if (crop.mode == 2) {
            // bilinear between texel centres, its blend squeezed into the
            // one window pixel each texel's edge falls in, at any scale
            vec2 scale = 1.0 / max(covered, vec2(1e-6));
            vec2 centred = texel - 0.5;
            vec2 blend = clamp((fract(centred) - 0.5) * scale + 0.5, 0.0, 1.0);
            at = (floor(centred) + 0.5 + blend) / size;
        }
        return sampled(eye, clamp(at, bounds.xy, bounds.zw)).rgb;
    }
    // each sample is bilinear, so it already averages its four texels
    ivec2 taps = ivec2(clamp(ceil(covered), 1.0, 4.0));
    vec3 sum = vec3(0.0);
    for (int y = 0; y < taps.y; y++) {
        for (int x = 0; x < taps.x; x++) {
            vec2 offset = ((vec2(x, y) + 0.5) / vec2(taps) - 0.5) * covered / size;
            sum += sampled(eye, clamp(at + offset, bounds.xy, bounds.zw)).rgb;
        }
    }
    return sum / float(taps.x * taps.y);
}

void main() {
#ifdef STEREO
    vec3 left = shown(0, crop.area, crop.bounds);
    vec3 right = shown(1, crop.right_area, crop.right_bounds);
    if (crop.swap != 0) {
        vec3 other = left;
        left = right;
        right = other;
    }
    if (crop.stereo == 1) {
        // Dubois' red and cyan projection, least squares over what the
        // glasses let through, with less ghosting than the plain split
        vec3 red = vec3(dot(left, vec3(0.437, 0.449, 0.164)), dot(left, vec3(-0.062, -0.062, -0.024)), dot(left, vec3(-0.048, -0.050, -0.017)));
        vec3 cyan = vec3(dot(right, vec3(-0.011, -0.032, -0.007)), dot(right, vec3(0.377, 0.761, 0.009)), dot(right, vec3(-0.026, -0.093, 1.234)));
        colour = vec4(clamp(red + cyan, 0.0, 1.0), 1.0);
    } else {
        // window rows from the top, even ones the left eye's
        colour = vec4((int(gl_FragCoord.y) & 1) == 0 ? left : right, 1.0);
    }
#else
    colour = vec4(shown(0, crop.area, crop.bounds), 1.0);
#endif
}
