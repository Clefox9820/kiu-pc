//! Anime4K's CNN upscaler (bloc97, MIT, see the header of the shader it
//! reads), run on the CPU to make a texture twice as big, once, off the
//! game's thread: see pack::Material::upscaled.
//!
//! the network is read from the mpv shader as Anime4K ships it, so another
//! of its sizes is a file swapped. each pass is a 3x3 or 1x1 convolution of
//! four channels, its inputs the picture or earlier passes, each taken whole
//! or as its positive or negative half (CReLU). the last pass turns each
//! texel's four channels into the 2x2 texels it becomes, added to the
//! picture made bigger by filtering.

use std::collections::HashMap;
use std::sync::OnceLock;

use rayon::prelude::*;

/// the shader the network is read from: VL, the largest of Anime4K's x2
/// CNNs that keeps a channel per color.
const SHADER: &str = include_str!("anime4k/Anime4K_Upscale_CNN_x2_VL.glsl");

/// how a pass takes a tensor: whole, or its positive or negative half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    Whole,
    Positive,
    Negative,
}

/// one tensor read at an offset, times four inputs by four outputs.
struct Term {
    input: usize,
    half: Half,
    dx: i32,
    dy: i32,
    /// by input channel, then output channel.
    weights: [[f32; 4]; 4],
}

struct Pass {
    output: usize,
    terms: Vec<Term>,
    bias: [f32; 4],
}

/// the passes in order. tensor 0 is the picture.
pub struct Network {
    passes: Vec<Pass>,
    tensors: usize,
    /// the tensors the last pass takes each color's 2x2 texels from, one
    /// for all of them or one a color.
    residuals: Vec<usize>,
}

/// the network, read once.
pub fn network() -> Option<&'static Network> {
    static NETWORK: OnceLock<Option<Network>> = OnceLock::new();
    NETWORK
        .get_or_init(|| {
            parse(SHADER).map_err(|error| log::warn!("could not read Anime4K's network, {error}")).ok()
        })
        .as_ref()
}

fn parse(source: &str) -> Result<Network, String> {
    let mut names: HashMap<&str, usize> = HashMap::from([("MAIN", 0)]);
    let mut passes = Vec::new();
    let mut residuals = Vec::new();
    for block in source.split("//!DESC").skip(1) {
        let (mut binds, mut save) = (Vec::new(), None);
        let mut reads: HashMap<&str, (usize, Half)> = HashMap::new();
        let (mut terms, mut bias) = (Vec::new(), [0.0; 4]);
        for line in block.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix("//!BIND ") {
                binds.push(name.trim());
            } else if let Some(name) = line.strip_prefix("//!SAVE ") {
                save = Some(name.trim());
            } else if let Some(define) = line.strip_prefix("#define ") {
                // go_0(x_off, y_off) (max(-(conv2d_tf_texOff(vec2(x_off, y_off))), 0.0))
                let alias = define.split(['(', ' ']).next().unwrap_or_default();
                let at = define.find("_texOff(").or_else(|| define.find("_tex(")).ok_or(format!("no texture in {line}"))?;
                let start = define[..at].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
                let tensor = *names.get(&define[start..at]).ok_or(format!("{} is read before it is made", &define[start..at]))?;
                let half = if define.contains("max(-(") {
                    Half::Negative
                } else if define.contains("max((") {
                    Half::Positive
                } else {
                    Half::Whole
                };
                reads.insert(alias, (tensor, half));
            } else if let Some(at) = line.find("mat4(") {
                let (matrix, rest) = line[at + 5..].split_once(')').ok_or(format!("no end to {line}"))?;
                let floats = numbers(matrix)?;
                let read = rest.trim_start().strip_prefix('*').ok_or(format!("nothing read in {line}"))?.trim();
                let alias = read.split(['(', ';']).next().unwrap_or_default().trim();
                let &(input, half) = reads.get(alias).ok_or(format!("{alias} is not defined"))?;
                let (dx, dy) = match read.split_once('(') {
                    Some((_, offsets)) => {
                        let offsets = numbers(offsets.split(')').next().unwrap_or_default())?;
                        (offsets[0] as i32, offsets[1] as i32)
                    }
                    None => (0, 0),
                };
                let floats: [f32; 16] = floats.try_into().map_err(|_| format!("not 16 weights in {line}"))?;
                let weights = std::array::from_fn(|input| std::array::from_fn(|output| floats[input * 4 + output]));
                terms.push(Term { input, half, dx, dy, weights });
            } else if line.starts_with("result +=") && line.contains("vec4(") {
                let inside = line.split_once("vec4(").and_then(|(_, rest)| rest.split_once(')')).map(|(inside, _)| inside);
                bias = numbers(inside.unwrap_or_default())?.try_into().map_err(|_| format!("not 4 biases in {line}"))?;
            }
        }
        match save {
            Some("MAIN") => {
                residuals = binds.iter().filter(|&&name| name != "MAIN").map(|name| names.get(name).copied().ok_or(format!("{name} is not made"))).collect::<Result<_, _>>()?;
            }
            Some(name) => {
                let output = names.len();
                names.insert(name, output);
                passes.push(Pass { output, terms, bias });
            }
            None => return Err("a pass saves nothing".to_owned()),
        }
    }
    if passes.is_empty() || residuals.is_empty() {
        return Err("no network in it".to_owned());
    }
    Ok(Network { passes, tensors: names.len(), residuals })
}

fn numbers(list: &str) -> Result<Vec<f32>, String> {
    list.split(',').map(|number| number.trim().parse::<f32>().map_err(|error| format!("{number:?}, {error}"))).collect()
}

/// the RGBA8 texels of a picture width by height made twice as big, RGBA8
/// rows from the top. the colors gain what the network adds, and so does
/// alpha, run through it as a grey picture of its own, as the shapes of a
/// HUD's letters and icons are most often in their alpha alone.
pub fn upscale(network: &Network, texels: &[[u8; 4]], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let colors = run(network, bleed(texels, w, h).iter().map(|texel| texel.map(|channel| channel as f32 / 255.0)).collect(), w, h);
    let alpha = texels
        .iter()
        .any(|texel| texel[3] < 255)
        .then(|| run(network, texels.iter().map(|texel| [texel[3] as f32 / 255.0, texel[3] as f32 / 255.0, texel[3] as f32 / 255.0, 0.0]).collect(), w, h));
    let source = &colors[0];
    let (ow, oh) = (w * 2, h * 2);
    let mut picture = vec![0u8; ow * oh * 4];
    picture.par_chunks_mut(ow * 4).enumerate().for_each(|(oy, row)| {
        for ox in 0..ow {
            let (x, y) = ((ox as f32 + 0.5) / 2.0 - 0.5, (oy as f32 + 0.5) / 2.0 - 0.5);
            let base = filtered(source, w, h, x, y);
            let (low, sub) = ((oy / 2) * w + ox / 2, (oy % 2) * 2 + ox % 2);
            let residual = |tensors: &[Vec<[f32; 4]>], channel: usize| tensors[network.residuals[channel.min(network.residuals.len() - 1)]][low][sub];
            for channel in 0..3 {
                row[ox * 4 + channel] = unit(base[channel] + residual(&colors, channel));
            }
            row[ox * 4 + 3] = match &alpha {
                // the grey picture's colors gain alike, their mean is alpha's
                Some(alpha) => unit(filtered(&alpha[0], w, h, x, y)[0] + (0..3).map(|channel| residual(alpha, channel)).sum::<f32>() / 3.0),
                None => 255,
            };
        }
    });
    picture
}

fn unit(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// every tensor of the network run on the picture, the picture first.
fn run(network: &Network, picture: Vec<[f32; 4]>, w: usize, h: usize) -> Vec<Vec<[f32; 4]>> {
    let mut tensors: Vec<Vec<[f32; 4]>> = vec![Vec::new(); network.tensors];
    tensors[0] = picture;
    for pass in &network.passes {
        let mut output = vec![[0.0f32; 4]; w * h];
        output.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, result) in row.iter_mut().enumerate() {
                *result = pass.bias;
                for term in &pass.terms {
                    let tx = (x as i32 + term.dx).clamp(0, w as i32 - 1) as usize;
                    let ty = (y as i32 + term.dy).clamp(0, h as i32 - 1) as usize;
                    let value = tensors[term.input][ty * w + tx];
                    let value = match term.half {
                        Half::Whole => value,
                        Half::Positive => value.map(|channel| channel.max(0.0)),
                        Half::Negative => value.map(|channel| (-channel).max(0.0)),
                    };
                    for (input, weights) in term.weights.iter().enumerate() {
                        for output in 0..4 {
                            result[output] += weights[output] * value[input];
                        }
                    }
                }
            }
        });
        tensors[pass.output] = output;
    }
    tensors
}

/// the texels with the color of each clear one taken from the ones around
/// it that show, again and again outward, as the network reads far around
/// each texel and the black most clear texels hold would darken the edges.
fn bleed(texels: &[[u8; 4]], w: usize, h: usize) -> Vec<[u8; 4]> {
    /// as far as the network looks around a texel.
    const REACH: usize = 16;
    let mut texels = texels.to_vec();
    let mut known: Vec<bool> = texels.iter().map(|texel| texel[3] > 0).collect();
    if known.iter().all(|&known| known) || !known.iter().any(|&known| known) {
        return texels;
    }
    for _ in 0..REACH {
        let mut filled = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if known[y * w + x] {
                    continue;
                }
                let (mut sum, mut count) = ([0u32; 3], 0);
                for ny in y.saturating_sub(1)..(y + 2).min(h) {
                    for nx in x.saturating_sub(1)..(x + 2).min(w) {
                        if known[ny * w + nx] {
                            let texel = texels[ny * w + nx];
                            (0..3).for_each(|channel| sum[channel] += texel[channel] as u32);
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    filled.push((y * w + x, sum.map(|total| (total / count) as u8)));
                }
            }
        }
        if filled.is_empty() {
            break;
        }
        for (index, [r, g, b]) in filled {
            texels[index] = [r, g, b, texels[index][3]];
            known[index] = true;
        }
    }
    texels
}

/// the texels filtered at x, y, in texels from the first one's center,
/// the edges carried on outward.
fn filtered(texels: &[[f32; 4]], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |x: f32, y: f32| texels[(y as i32).clamp(0, h as i32 - 1) as usize * w + (x as i32).clamp(0, w as i32 - 1) as usize];
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    std::array::from_fn(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sizes_of_the_network_are_read_whole() {
        let m = parse(include_str!("anime4k/Anime4K_Upscale_CNN_x2_M.glsl")).unwrap();
        assert_eq!((m.passes.len(), m.residuals.len()), (8, 1), "7 convolutions, the 1x1 one, luma alone");
        let vl = network().expect("the shipped network reads");
        assert_eq!((vl.passes.len(), vl.residuals.len()), (17, 3), "14 convolutions, three 1x1 ones, a residual per color");
        for pass in m.passes.iter().chain(&vl.passes) {
            assert!(!pass.terms.is_empty());
        }
        // the first pass reads the picture's colors alone, its alpha by nothing
        assert!(vl.passes[0].terms.iter().all(|term| term.input == 0 && term.weights[3] == [0.0; 4]));
    }

    /// a hard edge between two colors stays harder than filtering leaves
    /// it, alpha stays as it was where it is the same all around, and the
    /// picture is twice as big.
    #[test]
    fn an_edge_made_bigger_stays_sharp() {
        let (w, h) = (24u32, 24u32);
        let texels: Vec<[u8; 4]> =
            (0..w * h).map(|i| if i % w < w / 2 { [20, 20, 20, 255] } else { [235, 235, 235, 255] }).collect();
        let picture = upscale(network().unwrap(), &texels, w, h);
        assert_eq!(picture.len(), (w * h * 4 * 4) as usize);
        let row = (h as usize) * (w as usize * 2) * 4;
        let line = &picture[row..row + w as usize * 2 * 4];
        assert!(line.chunks(4).all(|texel| texel[3] == 255));
        // filtering leaves two texels between the colors on either side of
        // the edge, part of each: 20 + 215 * 0.25 and 20 + 215 * 0.75
        let between = |value: u8| (40..=215).contains(&value);
        let blurred = line.chunks(4).filter(|texel| between(texel[0])).count();
        assert!(blurred < 2, "{blurred} texels between the colors, {:?}", line.chunks(4).map(|texel| texel[0]).collect::<Vec<_>>());
    }

    /// a white letter's shape, in its alpha alone, stays as sharp made
    /// bigger as a color's edge does.
    #[test]
    fn an_edge_in_alpha_made_bigger_stays_sharp() {
        let (w, h) = (24u32, 24u32);
        let texels: Vec<[u8; 4]> = (0..w * h).map(|i| [255, 255, 255, if i % w < w / 2 { 0 } else { 255 }]).collect();
        let picture = upscale(network().unwrap(), &texels, w, h);
        let row = (h as usize) * (w as usize * 2) * 4;
        let line = &picture[row..row + w as usize * 2 * 4];
        let blurred = line.chunks(4).filter(|texel| (32..=223).contains(&texel[3])).count();
        assert!(blurred < 2, "{blurred} texels half clear, {:?}", line.chunks(4).map(|texel| texel[3]).collect::<Vec<_>>());
        assert!(line.chunks(4).all(|texel| texel[..3] == [255, 255, 255]), "white stays white");
    }

    #[test]
    fn clear_texels_take_the_colors_around_them() {
        // a red square in clear black
        let texels: Vec<[u8; 4]> = (0..64).map(|i| if (2..6).contains(&(i % 8)) && (2..6).contains(&(i / 8)) { [200, 0, 0, 255] } else { [0, 0, 0, 0] }).collect();
        let bled = bleed(&texels, 8, 8);
        assert!(bled.iter().all(|texel| texel[..3] == [200, 0, 0]), "{bled:?}");
        assert!(bled.iter().zip(&texels).all(|(bled, texel)| bled[3] == texel[3]), "alpha kept");
    }
}
