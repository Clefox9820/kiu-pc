#version 450
// SMAA 1x's second pass, blending weight calculation, over one screen's
// edges at its own size: searches along each edge for its ends and looks
// up how much of each pixel the edge's line covers, horizontal, vertical
// and diagonal, with SMAA's precomputed area and search textures.
// compiled with glslc or naga:
//   glslc smaa_weights.frag -o smaa_weights.frag.spv
// each picture is a combined image sampler, taken as its image and its
// sampler apart; naga will not have two at one binding, so the samplers
// are written at 14, 15 and 16 and moved onto 0, 1 and 2 after, see
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
// the edges, SMAA's area texture (160x560, RG8) and its search texture
// (64x16, R8), all sampled bilinearly
layout(set = 0, binding = 0) uniform texture2D edges_image;
layout(set = 0, binding = 14) uniform sampler edges_sampler;
layout(set = 0, binding = 1) uniform texture2D area_image;
layout(set = 0, binding = 15) uniform sampler area_sampler;
layout(set = 0, binding = 2) uniform texture2D search_image;
layout(set = 0, binding = 16) uniform sampler search_sampler;

// the same constants as the other passes, of which this reads only SMAA's
// metrics of the screen: 1 / width, 1 / height, width and height
layout(push_constant) uniform Pass {
    vec4 area;
    vec4 bounds;
    vec4 metrics;
} pass;

// SMAA's HIGH preset
const int MAX_SEARCH_STEPS = 16;
const int MAX_SEARCH_STEPS_DIAG = 8;
const float CORNER_ROUNDING = 25.0;
// what the area and search textures were made for
const float AREATEX_MAX_DISTANCE = 16.0;
const float AREATEX_MAX_DISTANCE_DIAG = 20.0;
const float AREATEX_WIDTH = 160.0;
const float AREATEX_HEIGHT = 560.0;
const float SEARCHTEX_WIDTH = 66.0;
const float SEARCHTEX_HEIGHT = 33.0;
const float SEARCHTEX_PACKED_WIDTH = 64.0;
const float SEARCHTEX_PACKED_HEIGHT = 16.0;

vec2 edges_at(vec2 t) {
    return textureLod(sampler2D(edges_image, edges_sampler), t, 0.0).rg;
}

// the edges a whole number of pixels from t
vec2 edges_offset(vec2 t, vec2 pixels) {
    return edges_at(t + pixels * pass.metrics.xy);
}

vec2 area_at(vec2 t) {
    return textureLod(sampler2D(area_image, area_sampler), t, 0.0).rg;
}

float search_at(vec2 t) {
    return textureLod(sampler2D(search_image, search_sampler), t, 0.0).r;
}

// ---------------------------------------------------------------------------
// diagonal searches

// decodes two edges from one bilinear fetch a quarter pixel off
vec2 decode_diag(vec2 e) {
    float r = e.x * abs(5.0 * e.x - 5.0 * 0.75);
    return round(vec2(r, e.y));
}

vec4 decode_diag4(vec4 e) {
    float r = e.x * abs(5.0 * e.x - 5.0 * 0.75);
    float b = e.z * abs(5.0 * e.z - 5.0 * 0.75);
    return round(vec4(r, e.y, b, e.w));
}

// how far a diagonal goes from t along dir, and whether it went on to the
// end of the search; e the edges where it stopped
vec2 search_diag1(vec2 t, vec2 dir, inout vec2 e) {
    vec2 at = t;
    float steps = -1.0;
    float going = 1.0;
    while (steps < float(MAX_SEARCH_STEPS_DIAG - 1) && going > 0.9) {
        at += pass.metrics.xy * dir;
        steps += 1.0;
        e = edges_at(at);
        going = dot(e, vec2(0.5));
    }
    return vec2(steps, going);
}

// the same for the other diagonal, fetching both its edges at once
vec2 search_diag2(vec2 t, vec2 dir, inout vec2 e) {
    vec2 at = t;
    at.x += 0.25 * pass.metrics.x;
    float steps = -1.0;
    float going = 1.0;
    while (steps < float(MAX_SEARCH_STEPS_DIAG - 1) && going > 0.9) {
        at += pass.metrics.xy * dir;
        steps += 1.0;
        e = decode_diag(edges_at(at));
        going = dot(e, vec2(0.5));
    }
    return vec2(steps, going);
}

// the area a diagonal of these lengths and crossing edges covers
vec2 area_diag(vec2 dist, vec2 e) {
    vec2 t = vec2(AREATEX_MAX_DISTANCE_DIAG) * e + dist;
    vec2 pixel = vec2(1.0 / AREATEX_WIDTH, 1.0 / AREATEX_HEIGHT);
    t = pixel * t + 0.5 * pixel;
    // diagonal areas are on the texture's right half
    t.x += 0.5;
    return area_at(t);
}

// the weights of the diagonals through this pixel
vec2 diag_weights(vec2 t, vec2 e) {
    vec2 weights = vec2(0.0);
    vec4 d = vec4(0.0);
    vec2 end = vec2(0.0);

    if (e.x > 0.0) {
        vec2 found = search_diag1(t, vec2(-1.0, 1.0), end);
        d.x = found.x + float(end.y > 0.9);
        d.z = found.y;
    }
    vec2 found = search_diag1(t, vec2(1.0, -1.0), end);
    d.y = found.x;
    d.w = found.y;

    if (d.x + d.y > 2.0) {
        // the crossing edges
        vec4 coords = vec4(-d.x + 0.25, d.x, d.y, -d.y - 0.25) * pass.metrics.xyxy + t.xyxy;
        vec2 left = edges_offset(coords.xy, vec2(-1.0, 0.0));
        vec2 right = edges_offset(coords.zw, vec2(1.0, 0.0));
        vec4 decoded = decode_diag4(vec4(left, right));
        vec4 c = vec4(decoded.y, decoded.x, decoded.w, decoded.z);
        vec2 cc = vec2(2.0) * c.xz + c.yw;
        // no crossing edge where the end was not found
        if (d.z >= 0.9) {
            cc.x = 0.0;
        }
        if (d.w >= 0.9) {
            cc.y = 0.0;
        }
        weights += area_diag(d.xy, cc);
    }

    found = search_diag2(t, vec2(-1.0, -1.0), end);
    d.x = found.x;
    d.z = found.y;
    if (edges_offset(t, vec2(1.0, 0.0)).x > 0.0) {
        found = search_diag2(t, vec2(1.0, 1.0), end);
        d.y = found.x + float(end.y > 0.9);
        d.w = found.y;
    } else {
        d.y = 0.0;
        d.w = 0.0;
    }

    if (d.x + d.y > 2.0) {
        vec4 coords = vec4(-d.x, -d.x, d.y, d.y) * pass.metrics.xyxy + t.xyxy;
        vec4 c;
        c.x = edges_offset(coords.xy, vec2(-1.0, 0.0)).y;
        c.y = edges_offset(coords.xy, vec2(0.0, -1.0)).x;
        vec2 right = edges_offset(coords.zw, vec2(1.0, 0.0));
        c.z = right.y;
        c.w = right.x;
        vec2 cc = vec2(2.0) * c.xz + c.yw;
        if (d.z >= 0.9) {
            cc.x = 0.0;
        }
        if (d.w >= 0.9) {
            cc.y = 0.0;
        }
        weights += area_diag(d.xy, cc).yx;
    }

    return weights;
}

// ---------------------------------------------------------------------------
// horizontal and vertical searches

// how much to add to a search's last step, from the edges it fetched
float search_length(vec2 e, float offset) {
    vec2 size = vec2(SEARCHTEX_WIDTH, SEARCHTEX_HEIGHT);
    vec2 packed_size = vec2(SEARCHTEX_PACKED_WIDTH, SEARCHTEX_PACKED_HEIGHT);
    // flipped vertically, the left and right cases each half of it across
    vec2 scale = size * vec2(0.5, -1.0) + vec2(-1.0, 1.0);
    vec2 bias = size * vec2(offset, 1.0) + vec2(0.5, -0.5);
    scale /= packed_size;
    bias /= packed_size;
    return search_at(scale * e + bias);
}

float search_x_left(vec2 t, float end) {
    vec2 at = t;
    vec2 e = vec2(0.0, 1.0);
    while (at.x > end && e.y > 0.8281 && e.x == 0.0) {
        e = edges_at(at);
        at = -vec2(2.0, 0.0) * pass.metrics.xy + at;
    }
    float offset = -(255.0 / 127.0) * search_length(e, 0.0) + 3.25;
    return pass.metrics.x * offset + at.x;
}

float search_x_right(vec2 t, float end) {
    vec2 at = t;
    vec2 e = vec2(0.0, 1.0);
    while (at.x < end && e.y > 0.8281 && e.x == 0.0) {
        e = edges_at(at);
        at = vec2(2.0, 0.0) * pass.metrics.xy + at;
    }
    float offset = -(255.0 / 127.0) * search_length(e, 0.5) + 3.25;
    return -pass.metrics.x * offset + at.x;
}

float search_y_up(vec2 t, float end) {
    vec2 at = t;
    vec2 e = vec2(1.0, 0.0);
    while (at.y > end && e.x > 0.8281 && e.y == 0.0) {
        e = edges_at(at);
        at = -vec2(0.0, 2.0) * pass.metrics.xy + at;
    }
    float offset = -(255.0 / 127.0) * search_length(e.yx, 0.0) + 3.25;
    return pass.metrics.y * offset + at.y;
}

float search_y_down(vec2 t, float end) {
    vec2 at = t;
    vec2 e = vec2(1.0, 0.0);
    while (at.y < end && e.x > 0.8281 && e.y == 0.0) {
        e = edges_at(at);
        at = vec2(0.0, 2.0) * pass.metrics.xy + at;
    }
    float offset = -(255.0 / 127.0) * search_length(e.yx, 0.5) + 3.25;
    return -pass.metrics.y * offset + at.y;
}

// the areas either side of an edge of these lengths and crossing edges
vec2 area(vec2 dist, float e1, float e2) {
    // rounding keeps bilinear filtering from creeping in
    vec2 t = vec2(AREATEX_MAX_DISTANCE) * round(4.0 * vec2(e1, e2)) + dist;
    vec2 pixel = vec2(1.0 / AREATEX_WIDTH, 1.0 / AREATEX_HEIGHT);
    return area_at(pixel * t + 0.5 * pixel);
}

// ---------------------------------------------------------------------------
// corners

vec2 horizontal_corners(vec2 weights, vec4 t, vec2 d) {
    vec2 left_right = step(d.xy, d.yx);
    vec2 rounding = (1.0 - CORNER_ROUNDING / 100.0) * left_right;
    // less blending in the middle of a line
    rounding /= left_right.x + left_right.y;
    float x = 1.0 - rounding.x * edges_offset(t.xy, vec2(0.0, 1.0)).x - rounding.y * edges_offset(t.zw, vec2(1.0, 1.0)).x;
    float y = 1.0 - rounding.x * edges_offset(t.xy, vec2(0.0, -2.0)).x - rounding.y * edges_offset(t.zw, vec2(1.0, -2.0)).x;
    return weights * clamp(vec2(x, y), 0.0, 1.0);
}

vec2 vertical_corners(vec2 weights, vec4 t, vec2 d) {
    vec2 left_right = step(d.xy, d.yx);
    vec2 rounding = (1.0 - CORNER_ROUNDING / 100.0) * left_right;
    rounding /= left_right.x + left_right.y;
    float x = 1.0 - rounding.x * edges_offset(t.xy, vec2(1.0, 0.0)).y - rounding.y * edges_offset(t.zw, vec2(1.0, 1.0)).y;
    float y = 1.0 - rounding.x * edges_offset(t.xy, vec2(-2.0, 0.0)).y - rounding.y * edges_offset(t.zw, vec2(-2.0, 1.0)).y;
    return weights * clamp(vec2(x, y), 0.0, 1.0);
}

void main() {
    vec4 m = pass.metrics;
    vec2 pixcoord = uv * m.zw;
    // offset to fetch four edges at once between them, then the ends of
    // the searches
    vec4 offset0 = m.xyxy * vec4(-0.25, -0.125, 1.25, -0.125) + uv.xyxy;
    vec4 offset1 = m.xyxy * vec4(-0.125, -0.25, -0.125, 1.25) + uv.xyxy;
    vec4 offset2 = m.xxyy * vec4(-2.0, 2.0, -2.0, 2.0) * float(MAX_SEARCH_STEPS) + vec4(offset0.xz, offset1.yw);

    vec4 weights = vec4(0.0);
    vec2 e = edges_at(uv);

    // an edge at the north
    if (e.y > 0.0) {
        // diagonals come first, they have edges north and west both
        vec2 diag = diag_weights(uv, e);
        weights.x = diag.x;
        weights.y = diag.y;
        if (diag.x == -diag.y) {
            vec3 coords;
            coords.x = search_x_left(offset0.xy, offset2.x);
            coords.y = offset1.y;
            vec2 d;
            d.x = coords.x;
            float e1 = edges_at(coords.xy).x;
            coords.z = search_x_right(offset0.zw, offset2.y);
            d.y = coords.z;
            // the distances in pixels
            d = abs(round(m.zz * d - pixcoord.xx));
            float e2 = edges_offset(coords.zy, vec2(1.0, 0.0)).x;
            // the area texture is compressed quadratically
            vec2 found = area(sqrt(d), e1, e2);
            found = horizontal_corners(found, vec4(coords.x, uv.y, coords.z, uv.y), d);
            weights.x = found.x;
            weights.y = found.y;
        } else {
            // a diagonal, which leaves out the vertical
            e.x = 0.0;
        }
    }

    // an edge at the west
    if (e.x > 0.0) {
        vec3 coords;
        coords.y = search_y_up(offset1.xy, offset2.z);
        coords.x = offset0.x;
        vec2 d;
        d.x = coords.y;
        float e1 = edges_at(coords.xy).y;
        coords.z = search_y_down(offset1.zw, offset2.w);
        d.y = coords.z;
        d = abs(round(m.ww * d - pixcoord.yy));
        float e2 = edges_offset(coords.xz, vec2(0.0, 1.0)).y;
        vec2 found = area(sqrt(d), e1, e2);
        found = vertical_corners(found, vec4(uv.x, coords.y, uv.x, coords.z), d);
        weights.z = found.x;
        weights.w = found.y;
    }

    colour = weights;
}
