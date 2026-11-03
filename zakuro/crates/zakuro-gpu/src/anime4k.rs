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
    /// the tensors it reads, in the order the shader binds them.
    inputs: Vec<usize>,
    output: usize,
    terms: Vec<Term>,
    bias: [f32; 4],
}

/// the passes in order. tensor 0 is the picture.
pub struct Network {
    passes: Vec<Pass>,
    tensors: usize,
    /// the tensors the last pass takes each color's 2x2 texels from, one
    /// for all of them or one a color, or for a Restore network, the one
    /// its last pass writes, added to the picture at its size.
    residuals: Vec<usize>,
    /// whether it restores the picture at its size rather than doubling it.
    restores: bool,
}

/// the network, read once.
pub fn network() -> Option<&'static Network> {
    read_once(&NETWORK_VL, SHADER)
}

/// M, the network the screen filter runs on the GPU every frame, four
/// times lighter than VL. its passes' shaders are made from the same file,
/// see the screen_shaders test.
pub const SCREEN_SHADER: &str = include_str!("anime4k/Anime4K_Upscale_CNN_x2_M.glsl");

/// the screen filter's network, read once.
pub fn screen_network() -> Option<&'static Network> {
    read_once(&NETWORK_M, SCREEN_SHADER)
}

/// Restore CNN M, which the screen filter runs before doubling, as Anime4K's
/// own fast mode does: it mends blurred and jagged lines at the picture's
/// size. its shaders are made as the upscaler's are.
pub const RESTORE_SHADER: &str = include_str!("anime4k/Anime4K_Restore_CNN_M.glsl");

/// the screen filter's Restore network, read once.
pub fn restore_network() -> Option<&'static Network> {
    read_once(&NETWORK_RESTORE, RESTORE_SHADER)
}

static NETWORK_VL: OnceLock<Option<Network>> = OnceLock::new();
static NETWORK_M: OnceLock<Option<Network>> = OnceLock::new();
static NETWORK_RESTORE: OnceLock<Option<Network>> = OnceLock::new();

fn read_once(network: &'static OnceLock<Option<Network>>, source: &str) -> Option<&'static Network> {
    network.get_or_init(|| parse(source).map_err(|error| log::warn!("could not read Anime4K's network, {error}")).ok()).as_ref()
}

impl Network {
    /// each pass's inputs, in the order its shader binds them, and output.
    /// tensor 0 is the picture.
    pub fn graph(&self) -> impl Iterator<Item = (&[usize], usize)> {
        self.passes.iter().map(|pass| (pass.inputs.as_slice(), pass.output))
    }

    /// how many tensors there are, the picture's among them.
    pub fn tensors(&self) -> usize {
        self.tensors
    }

    /// the tensors the last pass takes each color's 2x2 texels from.
    pub fn residuals(&self) -> &[usize] {
        &self.residuals
    }

    /// whether it restores the picture at its size, its last pass's
    /// output being the picture restored, rather than doubling it.
    pub fn restores(&self) -> bool {
        self.restores
    }
}

fn parse(source: &str) -> Result<Network, String> {
    let mut names: HashMap<&str, usize> = HashMap::from([("MAIN", 0)]);
    let mut passes = Vec::new();
    let (mut residuals, mut restores) = (Vec::new(), false);
    for block in source.split("//!DESC").skip(1) {
        let doubles = block.lines().next().is_some_and(|desc| desc.contains("Depth-to-Space"));
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
        let tensor = |name: &&str| names.get(name).copied().ok_or(format!("{name} is not made"));
        match save {
            Some("MAIN") if doubles => {
                residuals = binds.iter().filter(|&&name| name != "MAIN").map(tensor).collect::<Result<_, _>>()?;
            }
            Some(name) => {
                let inputs = binds.iter().map(tensor).collect::<Result<_, _>>()?;
                let output = names.len();
                if name == "MAIN" {
                    // a Restore network's last pass makes the picture again:
                    // a tensor of its own here, added to the picture
                    restores = true;
                    residuals = vec![output];
                } else {
                    names.insert(name, output);
                }
                passes.push(Pass { inputs, output, terms, bias });
            }
            None => return Err("a pass saves nothing".to_owned()),
        }
    }
    if passes.is_empty() || residuals.is_empty() {
        return Err("no network in it".to_owned());
    }
    let tensors = names.len() + usize::from(restores);
    Ok(Network { passes, tensors, residuals, restores })
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

/// the texels made twice as big by filtering alone, what the network adds
/// to, for tests to tell it did.
#[cfg(test)]
pub(crate) fn upscale_filtered(texels: &[[u8; 4]], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let source: Vec<[f32; 4]> = texels.iter().map(|texel| texel.map(|channel| channel as f32 / 255.0)).collect();
    (0..h * 2)
        .flat_map(|oy| (0..w * 2).map(move |ox| (ox, oy)))
        .flat_map(|(ox, oy)| filtered(&source, w, h, (ox as f32 + 0.5) / 2.0 - 0.5, (oy as f32 + 0.5) / 2.0 - 0.5).map(unit))
        .collect()
}

/// the texels restored by a Restore network at their size: what its last
/// pass makes added to their colors, alpha as it was. the screen filter
/// does this on the GPU, tests check it against this.
#[cfg(test)]
pub(crate) fn restore(network: &Network, texels: &[[u8; 4]], width: u32, height: u32) -> Vec<[u8; 4]> {
    let tensors = run(network, texels.iter().map(|texel| texel.map(|channel| channel as f32 / 255.0)).collect(), width as usize, height as usize);
    let added = &tensors[network.residuals[0]];
    texels
        .iter()
        .zip(added)
        .map(|(texel, added)| [0, 1, 2, 3].map(|channel| if channel == 3 { texel[3] } else { unit(texel[channel] as f32 / 255.0 + added[channel]) }))
        .collect()
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

    /// the screen filter's shaders made from a network's file, a pass a
    /// shader: each reads its tensors texel by texel, clamped at the edges
    /// as mpv's are, with mpv's own code for the pass, and an upscaler's
    /// last puts each texel's four channels on the 2x2 texels it becomes,
    /// over the picture made bigger.
    fn screen_shaders(source: &str, file: &str) -> Vec<String> {
        let network = parse(source).unwrap();
        let mut shaders = Vec::new();
        for block in source.split("//!DESC").skip(1) {
            let binds: Vec<&str> = block.lines().filter_map(|line| line.trim().strip_prefix("//!BIND ")).map(str::trim).collect();
            let doubles = block.lines().next().is_some_and(|desc| desc.contains("Depth-to-Space"));
            let mut glsl = format!(
                "#version 450\n// made from {file}, Anime4K by bloc97 (MIT, see that file),\n// by anime4k.rs's write_screen_shaders test: do not edit.\nlayout(location = 0) out vec4 fragment;\n"
            );
            for (binding, name) in binds.iter().enumerate() {
                glsl += &format!("layout(set = 0, binding = {binding}) uniform sampler2D {name};\n");
            }
            if doubles {
                let residuals = &binds[1..];
                let fetch = |channel: usize| format!("texelFetch({}, at / 2, 0)[sub]", residuals[channel.min(residuals.len() - 1)]);
                glsl += "void main() {\n    ivec2 at = ivec2(gl_FragCoord.xy);\n    int sub = (at.y % 2) * 2 + at.x % 2;\n";
                glsl += "    vec4 base = texture(MAIN, gl_FragCoord.xy / vec2(textureSize(MAIN, 0) * 2));\n";
                glsl += &format!("    fragment = vec4(clamp(base.rgb + vec3({}, {}, {}), 0.0, 1.0), 1.0);\n}}\n", fetch(0), fetch(1), fetch(2));
            } else {
                for name in &binds {
                    glsl += &format!(
                        "#define {name}_texOff(o) texelFetch({name}, clamp(ivec2(gl_FragCoord.xy) + ivec2(o), ivec2(0), textureSize({name}, 0) - 1), 0)\n#define {name}_tex(p) texelFetch({name}, ivec2(gl_FragCoord.xy), 0)\n#define {name}_pos 0\n"
                    );
                }
                for line in block.lines().skip(1).filter(|line| !line.trim_start().starts_with("//!") && !line.trim().is_empty()) {
                    glsl += line.trim_end();
                    glsl += "\n";
                }
                glsl += "void main() {\n    fragment = hook();\n}\n";
            }
            shaders.push(glsl);
        }
        let last = usize::from(!network.restores);
        assert_eq!(shaders.len(), network.passes.len() + last, "a shader a pass, and an upscaler's last");
        shaders
    }

    /// the screen filter's networks: the shaders' names, the file, and the
    /// network read from it.
    fn screen_networks() -> [(&'static str, &'static str, &'static Network); 2] {
        [
            ("restore", "Anime4K_Restore_CNN_M.glsl", restore_network().unwrap()),
            ("pass", "Anime4K_Upscale_CNN_x2_M.glsl", screen_network().unwrap()),
        ]
    }

    fn screen_shader_path(name: &str, index: usize) -> String {
        format!("{}/shaders/anime4k/{name}{index}.frag", env!("CARGO_MANIFEST_DIR"))
    }

    fn source(name: &str) -> &'static str {
        if name == "restore" { RESTORE_SHADER } else { SCREEN_SHADER }
    }

    /// writes the screen filter's shaders, which tools\glslang then
    /// compiles to SPIR-V: `glslang -V --target-env vulkan1.0 passN.frag -o
    /// passN.frag.spv`, and the same for restoreN.frag.
    #[test]
    #[ignore]
    fn write_screen_shaders() {
        std::fs::create_dir_all(format!("{}/shaders/anime4k", env!("CARGO_MANIFEST_DIR"))).unwrap();
        for (name, file, _) in screen_networks() {
            for (index, glsl) in screen_shaders(source(name), file).iter().enumerate() {
                std::fs::write(screen_shader_path(name, index), glsl).unwrap();
            }
        }
    }

    #[test]
    fn the_screen_shaders_are_made_from_the_networks_shipped() {
        for (name, file, network) in screen_networks() {
            let shaders = screen_shaders(source(name), file);
            for (index, glsl) in shaders.iter().enumerate() {
                let written = std::fs::read_to_string(screen_shader_path(name, index)).unwrap_or_default().replace("\r\n", "\n");
                assert_eq!(&written, glsl, "{name}{index}.frag is out of date: run write_screen_shaders and compile it again");
            }
            assert_eq!(network.graph().count() + usize::from(!network.restores()), shaders.len());
            assert_eq!(network.graph().next().unwrap().0, &[0], "the first pass reads the picture alone");
        }
        let restore = restore_network().unwrap();
        assert!(restore.restores() && !screen_network().unwrap().restores());
        assert_eq!(restore.graph().last().unwrap().0[0], 0, "its last pass reads the picture, to add to it");
    }

    /// Restore's last pass is added to the picture: a flat colour comes
    /// back near itself, alpha untouched.
    #[test]
    fn a_flat_picture_is_restored_near_itself() {
        let texels = vec![[120u8, 140, 160, 200]; 16 * 16];
        let restored = restore(restore_network().unwrap(), &texels, 16, 16);
        assert!(restored.iter().all(|texel| texel[3] == 200));
        let flat = texels[0];
        let worst = restored.iter().flat_map(|texel| (0..3).map(move |channel| texel[channel].abs_diff(flat[channel]))).max().unwrap();
        assert!(worst <= 8, "a flat colour moved by {worst}");
    }

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
