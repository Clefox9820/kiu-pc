//! Kid Icarus Uprising's top screen, as the port adds to it: with
//! ZAKURO_PORTRAITS, the dialogue portraits, the battle display and the
//! retro scenes the bottom screen draws are drawn again on the top screen,
//! and its subtitles are centered line by line over a shadow; on a top
//! screen shown wider than the console's, its HUD's 2D draws keep their
//! proportions. rasterize hands each draw to before_draw first.

use super::*;
use crate::blend::REG_BLEND_COLOR;

/// what a draw becomes after before_draw.
pub(super) enum Hook<'a> {
    /// drawn already, what rasterize returns for it.
    Drawn(Option<u32>),
    /// to draw as these vertices.
    Draw(Vertices<'a>),
}

/// what the port does with a draw before rasterize draws it: on a top
/// screen shown wider than the console's, a 2D draw's vertices kept in
/// proportion, put in squeezed; with ZAKURO_PORTRAITS, the subtitles drawn
/// centered over their shadow with the portraits under them, the text
/// buffer's lines and the bottom screen's portraits and battle display
/// taken for the top screen. the portraits and islands come back through
/// here as they are added, the subtitles too, and are squeezed then.
pub(super) fn before_draw<'a, M: GpuMemory>(
    registers: &[u32],
    memory: &mut M,
    resources: &mut Resources,
    vertices: Vertices<'a>,
    squeezed: &'a mut Option<Vec<Vertex>>,
) -> Hook<'a> {
    let portraits = portraits_on();
    // how far the title shook its top screen's HUD, which the battle
    // display taken from the bottom screen follows
    if portraits && !resources.kiu.adding && top_screen_2d(registers) {
        // arrays left to the GPU say nothing, the islands keep still then;
        // the widened screen's HUD has them decoded
        if let Some(shaken) = shaded_in_order(registers, vertices).as_deref().and_then(hud_shake) {
            resources.kiu.shake = shaken;
        }
    }
    let mut vertices = vertices;
    // subtitles are squeezed as they are added, centered, not twice
    let added = resources.kiu.adding || !(portraits && is_subtitle(registers));
    if resources.kiu.widening > 1.0 && added && top_screen_2d(registers) {
        // the CPU decodes their arrays, so their vertices say where they are
        if undecoded(vertices) {
            return Hook::Drawn(None);
        }
        let text = is_text(registers);
        let kiu = &mut resources.kiu;
        *squeezed = shaded_in_order(registers, vertices).and_then(|shaded| squeeze_hud(&shaded, kiu.widening, text, &mut kiu.hud_edges));
        let squeezed: &'a Option<Vec<Vertex>> = squeezed;
        if let Some(squeezed) = squeezed {
            vertices = Vertices::Shaded(squeezed);
        }
    }
    if !portraits || resources.kiu.adding {
        return Hook::Draw(vertices);
    }
    if on_top(registers) {
        note_top_buffer(registers, resources);
        if is_subtitle(registers) {
            return Hook::Drawn(draw_subtitles(registers, memory, resources, vertices));
        }
    } else if in_between(resources) {
        // the draw in between, into twins alone, takes nothing of the
        // title's: its text and bottom screen are the title's own draw's
    } else if is_text_buffer(registers) {
        // the CPU decodes its arrays, so its letters say where they are
        if undecoded(vertices) {
            return Hook::Drawn(None);
        }
        note_text(registers, resources, vertices);
    } else if resources.kiu.in_play && on_bottom(registers) {
        // the CPU decodes its arrays then, and its vertices say where it is
        if undecoded(vertices) {
            return Hook::Drawn(None);
        }
        if is_portrait(registers) {
            mirror_portrait(registers, resources, vertices);
        } else {
            mirror_hud(registers, resources, vertices);
        }
    }
    Hook::Draw(vertices)
}

/// whether a draw fills the top screen's 400x240, its viewport 200 by 120
/// halves, as its scene, HUD and subtitles do.
fn on_top(registers: &[u32]) -> bool {
    read_float24(registers, REG_VIEWPORT_WIDTH) == 120.0 && read_float24(registers, REG_VIEWPORT_HEIGHT) == 200.0
}

/// whether a draw is one of the top screen's 2D ones, with no depth test.
fn top_screen_2d(registers: &[u32]) -> bool {
    registers[REG_DEPTH_COLOR_MASK] & 1 == 0 && on_top(registers)
}

/// whether the GPU is to decode a draw's arrays, which leaves its vertices
/// unknown here; rasterize then has the CPU decode them and draw again.
fn undecoded(vertices: Vertices) -> bool {
    #[cfg(feature = "vulkan")]
    return matches!(vertices, Vertices::Raw { .. });
    #[cfg(not(feature = "vulkan"))]
    {
        let _ = vertices;
        false
    }
}

/// draws the subtitles: their shadow, then their text, each line centered
/// when the lines the title wrote are known, with the portraits under them
/// once a frame. both are drawn as added draws, so that on a widened top
/// screen they are squeezed alike. none when the CPU is to decode them.
fn draw_subtitles<M: GpuMemory>(registers: &[u32], memory: &mut M, resources: &mut Resources, vertices: Vertices) -> Option<u32> {
    // the CPU decodes its arrays, so its lines can be centered
    if undecoded(vertices) {
        return None;
    }
    resources.kiu.adding = true;
    // the portraits go under the subtitles, in the buffer that goes out,
    // once a frame
    let top = resources.kiu.top.map(|[address, ..]| address);
    if top == Some(registers[REG_COLOR_BUFFER_ADDRESS]) && !resources.kiu.flushed {
        resources.kiu.flushed = true;
        flush_portraits(memory, resources);
    }
    let centered = shaded_in_order(registers, vertices).and_then(|shaded| centered_subtitle(&shaded, &resources.kiu.text_lines));
    let vertices = centered.as_deref().map_or(vertices, Vertices::Shaded);
    draw_subtitle_shadow(registers, memory, resources, vertices);
    let drawn = rasterize(registers, memory, resources, vertices).unwrap_or(0);
    resources.kiu.adding = false;
    Some(drawn)
}

/// a 2D draw squeeze_hud squeezed this frame, by where it was across the
/// top screen's 400 and down its height in clip x, and where it went toward.
#[derive(Clone, Copy)]
pub(crate) struct EdgePiece {
    across: (f32, f32),
    down: (f32, f32),
    anchor: f32,
}

/// a piece overlapping or beside one already squeezed goes with it, within
/// this many pixels: a figure on its badge, the next piece of a line.
const HUD_TOUCH: f32 = 8.0;

/// and one on the same row as it within this many, a few letters.
const HUD_ROW_GAP: f32 = 32.0;

/// where a top-screen vertex is across the screen's 400 pixels from its
/// left edge; its clip y runs the other way, from 1 on the left to -1 on
/// the right.
fn across_top(vertex: &Vertex) -> f32 {
    200.0 * (1.0 - vertex.clip[1])
}

/// the lowest and the highest of the values.
fn span(values: impl Iterator<Item = f32>) -> (f32, f32) {
    values.fold((f32::MAX, f32::MIN), |(low, high), value| (low.min(value), high.max(value)))
}

/// whether a draw's vertices are all at w 1, as 2D draws put them.
fn flat(vertices: &[Vertex]) -> bool {
    !vertices.is_empty() && !vertices.iter().any(|vertex| (vertex.clip[3] - 1.0).abs() > 1e-4)
}

/// a 2D draw of the top screen, every vertex at w 1, kept in proportion on
/// a top screen shown factor times wider than the console's, when it is
/// not as wide as the screen: squeezed across by factor toward the screen's
/// edge it starts at, for a HUD's pieces along an edge, which keep their
/// places there and to one another, or else toward its own middle, for
/// what follows the scene, as markers do. a piece beside one squeezed
/// before goes where that one went, as the pieces of a line of text. the
/// title's text, which it can draw in several pieces, a few letters each
/// or parts of its text buffer, goes toward the screen's middle instead,
/// so the spaces between the pieces shrink with the letters. none for a
/// draw left as it is.
fn squeeze_hud(vertices: &[Vertex], factor: f32, text: bool, edges: &mut Vec<EdgePiece>) -> Option<Vec<Vertex>> {
    if factor <= 1.0 || !flat(vertices) {
        return None;
    }
    let (left, right) = span(vertices.iter().map(across_top));
    // as wide as the screen: a fade, a flash or the scene put together
    if right - left > 360.0 {
        return None;
    }
    let down = span(vertices.iter().map(|v| v.clip[0]));
    // down the 240 rows, clip x runs 2 for the lot
    let (reach, reach_down) = (HUD_TOUCH, HUD_TOUCH * 2.0 / 240.0);
    // or on the same row, a few letters along, as the end of a line past
    // a number drawn into it
    let same_row = |piece: &EdgePiece| (down.0 - piece.down.0).abs() < 2.0 / 120.0 && (down.1 - piece.down.1).abs() < 2.0 / 120.0;
    let touching = edges.iter().find(|piece| {
        let close = |reach: f32| left < piece.across.1 + reach && right > piece.across.0 - reach;
        close(reach) && down.0 < piece.down.1 + reach_down && down.1 > piece.down.0 - reach_down || same_row(piece) && close(HUD_ROW_GAP)
    });
    // markers that overlap go toward the same place too, a little off
    // theirs. a piece at the screen's very edge keeps to it; one beside a
    // piece squeezed before goes with that one, the end of a line near an
    // edge too; else one near an edge keeps to that edge
    let anchor = if left < 1.0 {
        0.0
    } else if right > 399.0 {
        400.0
    } else if let Some(piece) = touching {
        piece.anchor
    } else if left < 48.0 && right < 200.0 {
        0.0
    } else if right > 352.0 && left > 200.0 {
        400.0
    } else if text {
        200.0
    } else {
        (left + right) / 2.0
    };
    edges.push(EdgePiece { across: (left, right), down, anchor });
    Some(
        vertices
            .iter()
            .map(|vertex| {
                let mut squeezed = *vertex;
                let x = anchor + (across_top(vertex) - anchor) / factor;
                squeezed.clip[1] = 1.0 - x / 200.0;
                squeezed
            })
            .collect(),
    )
}

/// what the port keeps between draws for Kid Icarus Uprising's top screen:
/// what mirror_portrait and mirror_hud take from the bottom screen, what
/// the subtitles need, and how much wider the top screen shows.
#[derive(Default)]
pub(crate) struct KiuTop {
    /// how many times wider than the console's the top screen shows, which
    /// squeeze_hud takes back out of its 2D draws; 1 or less for not at all.
    pub(crate) widening: f32,
    /// the 2D pieces squeeze_hud kept against an edge this frame, see
    /// start_frame.
    hud_edges: Vec<EdgePiece>,
    /// where the last transfer to the top screen read from.
    pub(crate) top_source: Option<u32>,
    /// the top screen's color buffer registers, address, format and size,
    /// from the last draw into the buffer that transfer read.
    top: Option<[u32; 3]>,
    /// the right corner's portrait and the left one's.
    sides: [Side; 2],
    /// the battle display's draws the bottom screen made this frame that lie
    /// in an island, see ISLANDS.
    hud: Vec<PortraitDraw>,
    /// whether the game is in play, a battle, where its bottom screen's
    /// portraits are dialogue; in menus the same kind of picture is not.
    pub(crate) in_play: bool,
    /// whether this frame's portraits already went into the top screen,
    /// under the subtitles; else they go in before it goes out.
    pub(crate) flushed: bool,
    /// while the portraits or a shadow are being drawn, which are not the
    /// game's draws to look at.
    adding: bool,
    /// the lines of text the title last wrote into its text buffer, which
    /// the subtitles show, see note_text.
    text_lines: Vec<TextLine>,
    /// how far the title shook its top screen's HUD the last time it drew
    /// the health bar, across in pixels and down in clip x, see hud_shake.
    shake: (f32, f32),
    /// whether the bottom screen shows a retro scene this frame, see RETRO.
    retro: bool,
    /// the retro scene's draws the bottom screen made this frame, and the
    /// last ones taken with how much of them shows, fading in and out.
    retro_draws: Vec<PortraitDraw>,
    retro_shown: Option<Faded>,
}

impl KiuTop {
    /// a new frame of the title's starts, whose HUD pieces are squeezed
    /// afresh.
    pub(crate) fn start_frame(&mut self) {
        self.hud_edges.clear();
    }
}

/// a line of text in the title's 512x128 text buffer, in its pixels, rows
/// up from its bottom: the rows its letters take and how far across they go.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TextLine {
    bottom: f32,
    top: f32,
    left: f32,
    right: f32,
}

/// a draw of Kid Icarus Uprising's text: letters from its font, an A4
/// 256x256 picture, or its 512x128 text buffer the letters went into,
/// which it shows in more than one piece.
// matched by their shapes alone, as is_portrait is
fn is_text(registers: &[u32]) -> bool {
    let base = TEXTURE_UNIT_BASES[0];
    let font = registers[base + 1] == (256 << 16 | 256) && TextureFormat::from_raw(registers[base + 13]) == TextureFormat::A4;
    registers[REG_TEXTURE_CONFIG] & 1 != 0 && font || is_subtitle(registers)
}

/// a draw into Kid Icarus Uprising's 512x128 text buffer, where it writes
/// the letters its subtitles and dialogue box show.
fn is_text_buffer(registers: &[u32]) -> bool {
    read_float24(registers, REG_VIEWPORT_WIDTH) == 256.0 && read_float24(registers, REG_VIEWPORT_HEIGHT) == 64.0
}

/// keeps where the title writes letters into its text buffer, line by
/// line: a letter from its A4 font joins the line its rows overlap, and a
/// draw of anything else, the clearing that comes before new text, forgets
/// the lines it covers.
fn note_text(registers: &[u32], resources: &mut Resources, vertices: Vertices) {
    let Some(vertices) = shaded_in_order(registers, vertices) else { return };
    let viewport = (0.0, 0.0, 512.0, 128.0);
    let base = TEXTURE_UNIT_BASES[0];
    let letters = registers[REG_TEXTURE_CONFIG] & 1 != 0 && TextureFormat::from_raw(registers[base + 13]) == TextureFormat::A4;
    let boxes = vertices.as_chunks::<3>().0.iter().filter_map(|triangle| {
        let corners: Vec<Screen> = triangle.iter().filter_map(|vertex| to_screen(vertex, viewport)).collect();
        (corners.len() == 3).then(|| {
            let (xs, ys) = (corners.iter().map(|c| c.x), corners.iter().map(|c| c.y));
            TextLine {
                bottom: ys.clone().fold(f32::MAX, f32::min),
                top: ys.fold(f32::MIN, f32::max),
                left: xs.clone().fold(f32::MAX, f32::min),
                right: xs.fold(f32::MIN, f32::max),
            }
        })
    });
    let lines = &mut resources.kiu.text_lines;
    for found in boxes {
        if !letters {
            lines.retain(|line| line.top <= found.bottom || line.bottom >= found.top);
            continue;
        }
        let overlap = |line: &TextLine| line.top.min(found.top) - line.bottom.max(found.bottom);
        match lines.iter_mut().find(|line| overlap(line) * 2.0 >= found.top - found.bottom) {
            Some(line) => {
                *line = TextLine {
                    bottom: line.bottom.min(found.bottom),
                    top: line.top.max(found.top),
                    left: line.left.min(found.left),
                    right: line.right.max(found.right),
                }
            }
            None => lines.push(found),
        }
    }
}

/// the subtitles' vertices with each line of text in the middle of the top
/// screen: every quad of the draw, the text and the title's own shadow, cut
/// into one slice per line across the text buffer's rows, each slid along
/// the screen by what its line needs. none when the draw is not quads
/// across the lines the buffer holds.
fn centered_subtitle(vertices: &[Vertex], lines: &[TextLine]) -> Option<Vec<Vertex>> {
    if lines.is_empty() || vertices.is_empty() || !vertices.len().is_multiple_of(6) {
        return None;
    }
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    // the quads' corners, by their texture coordinates, low and high u and v
    let corners = |quad: &[Vertex]| -> Option<[[Vertex; 2]; 2]> {
        let (us, vs) = (quad.iter().map(|v| v.texcoords[0][0]), quad.iter().map(|v| v.texcoords[0][1]));
        let (u0, u1) = (us.clone().fold(f32::MAX, f32::min), us.fold(f32::MIN, f32::max));
        let (v0, v1) = (vs.clone().fold(f32::MAX, f32::min), vs.fold(f32::MIN, f32::max));
        let at = |u: f32, v: f32| quad.iter().find(|x| near(x.texcoords[0][0], u) && near(x.texcoords[0][1], v)).copied();
        Some([[at(u0, v0)?, at(u0, v1)?], [at(u1, v0)?, at(u1, v1)?]])
    };
    let first = corners(&vertices[..6])?;
    let [u0, v0] = first[0][0].texcoords[0];
    let [u1, v1] = first[1][1].texcoords[0];
    // the lines the quads show, low rows first, in texture terms
    let mut shown: Vec<TextLine> = lines
        .iter()
        .map(|line| TextLine { bottom: line.bottom / 128.0, top: line.top / 128.0, left: line.left / 512.0, right: line.right / 512.0 })
        .filter(|line| (line.bottom + line.top) / 2.0 > v0 && (line.bottom + line.top) / 2.0 < v1)
        .collect();
    if shown.is_empty() {
        return None;
    }
    shown.sort_by(|a, b| a.bottom.total_cmp(&b.bottom));
    // each line's slide, in clip-space units of the top screen's 400 across,
    // which a vertex's clip y holds: its middle goes to the screen's middle
    let ndc_y = |vertex: &Vertex| vertex.clip[1] / vertex.clip[3];
    let slides: Vec<f32> = shown
        .iter()
        .map(|line| {
            let middle = ((line.left + line.right) / 2.0 - u0) / (u1 - u0);
            let low = ndc_y(&first[0][0]) + (ndc_y(&first[1][0]) - ndc_y(&first[0][0])) * middle;
            -low
        })
        .collect();
    // where each slice starts and ends in v: the quad's ends, and halfway
    // between one line and the next
    let mut cuts = vec![v0];
    cuts.extend(shown.windows(2).map(|pair| (pair[0].top + pair[1].bottom) / 2.0));
    cuts.push(v1);
    let mut out = Vec::with_capacity(vertices.len() * shown.len());
    for quad in vertices.as_chunks::<6>().0 {
        let edges = corners(quad)?;
        for (slice, slide) in slides.iter().enumerate() {
            let (from, to) = ((cuts[slice] - v0) / (v1 - v0), (cuts[slice + 1] - v0) / (v1 - v0));
            for vertex in quad {
                let side = &edges[!near(vertex.texcoords[0][0], edges[0][0].texcoords[0][0]) as usize];
                let high = !near(vertex.texcoords[0][1], side[0].texcoords[0][1]);
                let mut moved = side[0].lerp(&side[1], if high { to } else { from });
                moved.clip[1] += slide * moved.clip[3];
                out.push(moved);
            }
        }
    }
    Some(out)
}

/// a corner of the top screen and the portraits fading in and out there.
#[derive(Default)]
struct Side {
    /// the draws of the portrait the game drew this frame.
    incoming: Vec<PortraitDraw>,
    /// the portrait shown, and the one before it while it fades out.
    shown: Option<Faded>,
    leaving: Option<Faded>,
}

/// a portrait's draws, moved into the top screen's buffer, and how much of
/// it shows.
struct Faded {
    draws: Vec<PortraitDraw>,
    alpha: f32,
}

/// one draw of a portrait, its registers pointed at the top screen and its
/// vertices already shaded, so it can be drawn again after the game stops.
struct PortraitDraw {
    registers: Vec<u32>,
    vertices: Vec<Vertex>,
}

impl PortraitDraw {
    /// the picture it shows, its texture's address.
    fn picture(&self) -> u32 {
        self.registers[TEXTURE_UNIT_BASES[0] + 4]
    }
}

// a test's own ZAKURO_PORTRAITS, which the tests running beside it on other
// threads do not see.
#[cfg(test)]
thread_local! {
    static PORTRAITS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// whether a draw is the one in between alone, into twins, which takes
/// nothing for the top screen, the title's own draw did, unless every twin
/// is kept, see twin_all.
fn in_between(resources: &Resources) -> bool {
    #[cfg(feature = "vulkan")]
    return resources.hardware.as_ref().is_some_and(|hardware| hardware.pass == Pass::Twin && hardware.twin_cuts);
    #[cfg(not(feature = "vulkan"))]
    {
        let _ = resources;
        false
    }
}

/// whether ZAKURO_PORTRAITS=1 asks for the bottom screen's portraits on the
/// top screen.
pub(crate) fn portraits_on() -> bool {
    #[cfg(test)]
    if PORTRAITS.with(std::cell::Cell::get) {
        return true;
    }
    static PORTRAITS_ON: crate::env::EnvFlag = crate::env::EnvFlag::new("ZAKURO_PORTRAITS");
    PORTRAITS_ON.on()
}

/// a dialogue portrait as Kid Icarus Uprising draws it, an Etc1 256x256
/// picture on the whole bottom screen.
// matched by its shape alone; another game drawing the same way would need
// a list by title
fn is_portrait(registers: &[u32]) -> bool {
    let base = TEXTURE_UNIT_BASES[0];
    on_bottom(registers)
        && registers[base + 1] == (256 << 16 | 256)
        && TextureFormat::from_raw(registers[base + 13]) == TextureFormat::Etc1
}

/// a textured draw over the whole bottom screen's 240x320, the way the
/// title draws its pictures there.
fn on_bottom(registers: &[u32]) -> bool {
    read_float24(registers, REG_VIEWPORT_WIDTH) == 120.0
        && read_float24(registers, REG_VIEWPORT_HEIGHT) == 160.0
        && registers[REG_TEXTURE_CONFIG] & 1 != 0
}

/// Kid Icarus Uprising's subtitles: its 512x128 RGBA8 text buffer drawn on
/// the top screen.
fn is_subtitle(registers: &[u32]) -> bool {
    let base = TEXTURE_UNIT_BASES[0];
    registers[REG_TEXTURE_CONFIG] & 1 != 0
        && registers[base + 1] == (512 << 16 | 128)
        && TextureFormat::from_raw(registers[base + 13]) == TextureFormat::Rgba8
}

/// draws the subtitles' text again first, black and partly see-through, a
/// pixel down and right of it, a shadow that keeps it readable over bright
/// scenes and the portraits.
fn draw_subtitle_shadow<M: GpuMemory>(registers: &[u32], memory: &mut M, resources: &mut Resources, vertices: Vertices) {
    /// how dark the shadow is, out of 255.
    const SHADOW_ALPHA: u32 = 0xC0;
    let mut shadow = registers.to_vec();
    // the first stage: constant black, with the text's alpha times the
    // constant's; the rest pass it on
    for (stage, &base) in TEV_STAGES.iter().enumerate() {
        let (source, combiner) = match stage {
            0 => (0xE | 3 << 16 | 0xE << 20, 1 << 16),
            _ => (0xF | 0xF << 16, 0),
        };
        shadow[base..base + 5].copy_from_slice(&[source, 0, combiner, SHADOW_ALPHA << 24, 0]);
    }
    // the top screen's buffer runs across its rows from the screen's
    // bottom, so a row lower is x - 1 and a column right y + 1
    let xy = registers[REG_VIEWPORT_XY];
    let shifted = |value: u32, by: i32| ((((value & 0x3FF) << 22) as i32 >> 22) + by) as u32 & 0x3FF;
    shadow[REG_VIEWPORT_XY] = shifted(xy, -1) | shifted(xy >> 16, 1) << 16;
    rasterize(&shadow, memory, resources, vertices);
}

/// the texture combiner's six stages, each five registers from there.
const TEV_STAGES: [usize; 6] = [0x0C0, 0x0C8, 0x0D0, 0x0D8, 0x0F0, 0x0F8];

/// remembers the top screen's color buffer, the one the last transfer to
/// the top screen read from, as a draw into it shows it.
fn note_top_buffer(registers: &[u32], resources: &mut Resources) {
    let Some(source) = resources.kiu.top_source else { return };
    let dimensions = registers[REG_FRAMEBUFFER_DIMENSIONS];
    let (width, height) = (dimensions & 0x7FF, ((dimensions >> 12) & 0x3FF) + 1);
    let format = ColorFormat::from_color_buffer((registers[REG_COLOR_BUFFER_FORMAT] >> 16) & 7);
    let start = loc_register(registers, REG_COLOR_BUFFER_ADDRESS);
    if (start..start + width * height * format.bytes_per_pixel() as u32).contains(&source) {
        resources.kiu.top = Some([
            registers[REG_COLOR_BUFFER_ADDRESS],
            registers[REG_COLOR_BUFFER_FORMAT],
            dimensions,
        ]);
    }
}

/// the part of a draw on the bottom screen, lowest and highest corners, x
/// across its 240 rows from the screen's bottom edge, y along them from its
/// left edge; none when none of it is on the screen.
fn bottom_box(vertices: &[Vertex]) -> Option<([f32; 2], [f32; 2])> {
    let viewport = (0.0, 0.0, 240.0, 320.0);
    let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
    for screen in vertices.iter().filter(|vertex| inside(vertex)).filter_map(|vertex| to_screen(vertex, viewport)) {
        low = [low[0].min(screen.x), low[1].min(screen.y)];
        high = [high[0].max(screen.x), high[1].max(screen.y)];
    }
    let (low, high) = ([low[0].max(0.0), low[1].max(0.0)], [high[0].min(240.0), high[1].min(320.0)]);
    (low[0] < high[0] && low[1] < high[1]).then_some((low, high))
}

/// a bottom-screen draw's registers pointed at the top screen's buffer, the
/// whole bottom screen scaled by scale with its corner at x, y there, and
/// no depth, the top screen's buffer has none of the bottom one's.
fn onto_top(registers: &[u32], [address, format, dimensions]: [u32; 3], scale: f32, x: f32, y: f32) -> Vec<u32> {
    let mut patched = registers.to_vec();
    patched[REG_COLOR_BUFFER_ADDRESS] = address;
    patched[REG_COLOR_BUFFER_FORMAT] = format;
    patched[REG_FRAMEBUFFER_DIMENSIONS] = dimensions;
    patched[REG_VIEWPORT_WIDTH] = float24(120.0 * scale);
    patched[REG_VIEWPORT_HEIGHT] = float24(160.0 * scale);
    patched[REG_VIEWPORT_XY] = (x.round() as i32 as u32 & 0x3FF) | ((y.round() as i32 as u32 & 0x3FF) << 16);
    patched[REG_DEPTH_COLOR_MASK] &= !(1 | 1 << 12);
    patched[REG_DEPTH_STENCIL_WRITE] = 0;
    patched[REG_DEPTH_BUFFER_FORMAT] &= !3;
    patched
}

/// takes a bottom-screen portrait for the top screen's buffer, smaller, in
/// the lower corner on its side, beside the subtitles. flush_portraits
/// draws it there.
fn mirror_portrait(registers: &[u32], resources: &mut Resources, vertices: Vertices) {
    /// how big the portrait is on the top screen, and how far from its side:
    /// none, the bottom screen's side edge cuts some, and that cut goes on
    /// the top screen's side edge so it does not show either.
    const SCALE: f32 = 0.4;
    const MARGIN: f32 = 0.0;
    let Some(top) = resources.kiu.top else { return };
    resources.textures.note_portrait(loc_register(registers, TEXTURE_UNIT_BASES[0] + 4));
    let Some(vertices) = shaded_in_order(registers, vertices) else { return };
    // the screen's edge cuts it, so that edge goes on the top screen's
    // bottom edge and the cut does not show
    let Some((low, high)) = bottom_box(&vertices) else { return };
    // its lowest x on the bottom edge, and its y against the end of its own
    // side of the top screen's 400; y runs leftward from the screen's right
    // edge on both screens, so a low y is on the right
    let right = (low[1] + high[1]) / 2.0 < 160.0;
    let x = -low[0] * SCALE;
    let y = match right {
        true => MARGIN - low[1] * SCALE,
        false => 400.0 - MARGIN - high[1] * SCALE,
    };
    let mut registers = onto_top(registers, top, SCALE, x, y);
    // the title's pictures stop at the bottom screen's top edge, some cut
    // there, Medusa's snakes; on the top screen that edge fades out
    let vertices = faded_top(registers[REG_PRIMITIVE_CONFIG] >> 8 & 3, &vertices);
    registers[REG_PRIMITIVE_CONFIG] &= !(3 << 8);
    resources.kiu.sides[!right as usize].incoming.push(PortraitDraw { registers, vertices });
}

/// how many of the bottom screen's rows below its top edge a portrait fades
/// out over on the top screen.
const PORTRAIT_FADE: f32 = 24.0;

/// a bottom-screen draw's triangles, as a list, fading out over the last
/// PORTRAIT_FADE rows before the bottom screen's top edge, by their alpha:
/// each triangle is cut along the band's lower edge, so the band has
/// corners of its own and its alpha runs from whole to none across it alone.
// it fades by the vertices' alpha, so only a draw that blends by it fades,
// as Kid Icarus Uprising's portraits do
fn faded_top(topology: u32, vertices: &[Vertex]) -> Vec<Vertex> {
    // where an edge crosses the band's lower edge, in clip space, so the
    // cut is where it is however far the vertex is
    let edge = (240.0 - PORTRAIT_FADE) / 120.0 - 1.0;
    let below = |vertex: &Vertex| edge * vertex.clip[3] - vertex.clip[0];
    let mut faded = Vec::with_capacity(vertices.len() * 2);
    for (a, b, c) in assemble(topology, vertices.len()) {
        let [under, over] = split(&[vertices[a], vertices[b], vertices[c]], below);
        // the part below the band's edge, then the part in it
        fan(&over, &mut faded);
        fan(&under, &mut faded);
    }
    for vertex in &mut faded {
        let row = (vertex.clip[0] / vertex.clip[3] * 0.5 + 0.5) * 240.0;
        vertex.color[3] *= ((240.0 - row) / PORTRAIT_FADE).clamp(0.0, 1.0);
    }
    faded
}

/// the polygon's part where distance is at most 0 and its part where it is
/// at least 0, each with the points where an edge crosses 0.
fn split(polygon: &[Vertex], distance: impl Fn(&Vertex) -> f32) -> [Vec<Vertex>; 2] {
    let mut sides = [Vec::with_capacity(polygon.len() + 1), Vec::with_capacity(polygon.len() + 1)];
    for (i, current) in polygon.iter().enumerate() {
        let next = &polygon[(i + 1) % polygon.len()];
        let (d0, d1) = (distance(current), distance(next));
        if d0 <= 0.0 {
            sides[0].push(*current);
        }
        if d0 >= 0.0 {
            sides[1].push(*current);
        }
        if d0 * d1 < 0.0 {
            let cut = current.lerp(next, d0 / (d0 - d1));
            sides[0].push(cut);
            sides[1].push(cut);
        }
    }
    sides
}

/// a convex polygon's triangles, as a list, fanned out from its first
/// corner.
fn fan(polygon: &[Vertex], triangles: &mut Vec<Vertex>) {
    for i in 1..polygon.len().saturating_sub(1) {
        triangles.extend([polygon[0], polygon[i], polygon[i + 1]]);
    }
}

/// Kid Icarus Uprising's health bar's inner piece at rest on the top
/// screen: 90 pixels across from 32 from its left edge, and from 0.8083 to
/// 0.85 down in clip x.
const HEALTH_BAR_WIDTH: f32 = 90.0;
const HEALTH_BAR_LEFT: f32 = 32.0;
const HEALTH_BAR_TOP: f32 = 0.8083;
const HEALTH_BAR_BOTTOM: f32 = 0.85;

/// how far a top-screen 2D draw that is Kid Icarus Uprising's health bar,
/// see HEALTH_BAR_WIDTH, has been moved from its place at rest: across in
/// pixels and down in clip x, which the title does to the whole HUD when
/// Pit is hit. none for another draw.
// the bar's place for this one title, as ISLANDS has
fn hud_shake(vertices: &[Vertex]) -> Option<(f32, f32)> {
    if !flat(vertices) {
        return None;
    }
    let (left, right) = span(vertices.iter().map(across_top));
    let (top, bottom) = span(vertices.iter().map(|v| v.clip[0]));
    let near = |a: f32, b: f32, by: f32| (a - b).abs() < by;
    let bar = near(right - left, HEALTH_BAR_WIDTH, 0.5) && near(bottom - top, HEALTH_BAR_BOTTOM - HEALTH_BAR_TOP, 0.002);
    (bar && near(left, HEALTH_BAR_LEFT, 12.0)).then_some((left - HEALTH_BAR_LEFT, top - HEALTH_BAR_TOP))
}

/// moves a top-screen draw's vertices across by pixels and down by clip x.
fn shake(vertices: &mut [Vertex], (across, down): (f32, f32)) {
    for vertex in vertices {
        vertex.clip[1] -= across / 200.0 * vertex.clip[3];
        vertex.clip[0] += down * vertex.clip[3];
    }
}

/// a part of the bottom screen's battle display shown on the top screen:
/// where it is on the bottom screen, left, top, right and bottom from its
/// top left, where its top left goes on the top screen, and its scale.
struct Island {
    from: [f32; 4],
    to: [f32; 2],
    scale: f32,
}

/// Kid Icarus Uprising's battle display: the counters (intensity, enemies
/// defeated, hearts) under the score, and the powers (the orbs in the air,
/// the power palette on foot) under the health bar.
// fixed places for this one title; another game with islands would need a
// table by title
const ISLANDS: [Island; 2] = [
    // against the screen's edges, as on the bottom screen
    Island { from: [196.0, 0.0, 320.0, 52.0], to: [400.0 - 124.0 * 0.6, 44.0], scale: 0.6 },
    // the orbs and the palette sit in the island's lower half, from 194 down,
    // so it starts above the health bar's foot for them to land just below it
    Island { from: [0.0, 150.0, 170.0, 240.0], to: [0.0, 16.0], scale: 0.5 },
];

/// Kid Icarus Uprising's retro scenes, Medusa and Hewdraw as the NES game
/// drew them, in a band across the bottom screen, see is_retro: on the top screen at 0.6 in its middle, so it shows who
/// speaks, its edges fading out, see feathered.
const RETRO: Island = Island { from: [0.0, 56.0, 320.0, 184.0], to: [200.0 - 320.0 * 0.6 / 2.0, 120.0 - 128.0 * 0.6 / 2.0], scale: 0.6 };

/// how many of the bottom screen's pixels across and down a retro scene's
/// picture takes at least to start one, see mirror_hud: Medusa's band takes
/// 220 by 128, Hewdraw's sprite 96 by 124, the battle display's own
/// pictures of those formats about 46 by 39 at most.
const RETRO_SIZE: f32 = 64.0;

/// how many of the top screen's pixels the retro scene fades out over at
/// each of its edges.
const RETRO_FEATHER: f32 = 10.0;

/// a top-screen draw's triangles, as a list, each cut a feather's width in
/// from the edges of bounds, low and high corners in clip x and y over w,
/// the feather given the same way, and fading out from there to the edges
/// by their vertices' alpha: none at an edge, all of it a feather in.
fn feathered(topology: u32, vertices: &[Vertex], (low, high): ([f32; 2], [f32; 2]), feather: [f32; 2]) -> Vec<Vertex> {
    let at = |vertex: &Vertex, axis: usize| vertex.clip[axis] / vertex.clip[3];
    let mut polygons: Vec<Vec<Vertex>> = assemble(topology, vertices.len()).into_iter().map(|(a, b, c)| vec![vertices[a], vertices[b], vertices[c]]).collect();
    for axis in 0..2 {
        for line in [low[axis] + feather[axis], high[axis] - feather[axis]] {
            polygons = polygons
                .into_iter()
                .flat_map(|polygon| split(&polygon, |vertex| at(vertex, axis) - line).into_iter().filter(|side| side.len() >= 3))
                .collect();
        }
    }
    let mut cut = Vec::with_capacity(polygons.len() * 6);
    for polygon in polygons {
        fan(&polygon, &mut cut);
    }
    for vertex in &mut cut {
        let mut keep = 1.0f32;
        for axis in 0..2 {
            let value = at(vertex, axis);
            keep = keep.min(((value - low[axis]) / feather[axis]).clamp(0.0, 1.0));
            keep = keep.min(((high[axis] - value) / feather[axis]).clamp(0.0, 1.0));
        }
        vertex.color[3] *= keep;
    }
    cut
}

/// a draw's registers made to blend by the alpha its vertices carry: the
/// combiner's last stage takes the stage before's alpha times the vertex
/// colour's, its colours as they were; the result goes over the buffer by
/// that alpha, the buffer's alpha kept, with no alpha test to cut a faded
/// edge off. a draw that blends already is left as it is: the game's retro
/// scenes blend by their texture's alpha times their vertices'.
// a title whose last stage works on alpha loses that, which none of Kid
// Icarus Uprising's retro scenes does
fn by_vertex_alpha(registers: &[u32]) -> Vec<u32> {
    let mut blended = registers.to_vec();
    if registers[REG_COLOR_OPERATION] & 0x100 != 0 {
        return blended;
    }
    let last = TEV_STAGES[5];
    // sources: the previous stage's alpha and the vertex colour's, their
    // alphas, multiplied, at 1x
    blended[last] = (blended[last] & 0xFFFF) | 0xF << 16;
    blended[last + 1] &= !(0xFFF << 12);
    blended[last + 2] = (blended[last + 2] & 0xFFFF) | 1 << 16;
    blended[last + 4] &= !(3 << 16);
    blended[REG_COLOR_OPERATION] |= 0x100;
    blended[REG_BLEND_FUNC] = 6 << 16 | 7 << 20 | 1 << 28;
    blended[REG_ALPHA_TEST] &= !1;
    blended
}

/// how much of the retro scene shows at most on the top screen, so the
/// scene behind it shows through a little, and how much more or less of it
/// each frame as it fades in and out.
const RETRO_ALPHA: f32 = 0.85;
const RETRO_STEP: f32 = 1.0 / 8.0;

/// how much of the retro scene shows the frame after one with alpha of it,
/// whether the game drew it this frame or not.
fn retro_alpha(alpha: f32, drawn: bool) -> f32 {
    match drawn {
        true => (alpha + RETRO_STEP).min(RETRO_ALPHA),
        false => (alpha - RETRO_STEP).max(0.0),
    }
}

/// a draw of a retro scene's picture, see RETRO: in play, the bottom
/// screen's pictures in RGB565 or RGBA5551 are the NES game's, Medusa's
/// band and Hewdraw's sprite among them; its battle display's are ETC1,
/// L4, LA4, L8 or RGBA4.
// told by format alone, as seen in chapters 1 to 3; a picture of those
// formats there that is not retro shows on the top screen too
fn is_retro(registers: &[u32]) -> bool {
    let base = TEXTURE_UNIT_BASES[0];
    registers[REG_TEXTURE_CONFIG] & 1 != 0
        && matches!(TextureFormat::from_raw(registers[base + 13]), TextureFormat::Rgb565 | TextureFormat::Rgba5551)
}

/// takes a bottom-screen draw that lies in one of the islands for the top
/// screen's buffer, where that island goes, for flush_portraits to draw.
fn mirror_hud(registers: &[u32], resources: &mut Resources, vertices: Vertices) {
    let Some(top) = resources.kiu.top else { return };
    let Some(vertices) = shaded_in_order(registers, vertices) else { return };
    let Some((low, high)) = bottom_box(&vertices) else { return };
    // in screen terms, left, top, right and bottom, y down: the buffer's
    // rows run up from the screen's bottom edge and leftward from its right
    let [left, upper, right, lower] = [320.0 - high[1], 240.0 - high[0], 320.0 - low[1], 240.0 - low[0]];
    const SLACK: f32 = 2.0;
    let inside = |island: &&Island| {
        let [x0, y0, x1, y1] = island.from;
        left >= x0 - SLACK && upper >= y0 - SLACK && right <= x1 + SLACK && lower <= y1 + SLACK
    };
    // a retro scene starts with a big picture in the band, then its smaller
    // ones go with it, the names over it; the battle display's own small
    // pictures of those formats do not start one
    let retro = is_retro(registers) && inside(&&RETRO);
    resources.kiu.retro |= retro && right - left >= RETRO_SIZE && lower - upper >= RETRO_SIZE;
    let in_retro = resources.kiu.retro && retro;
    let Some(island) = (if in_retro { Some(&RETRO) } else { ISLANDS.iter().find(inside) }) else {
        return;
    };
    // the island's top left on the bottom screen lands on its place on the
    // top screen; in buffer terms x runs up from the screen's bottom edge
    // and y leftward from its right edge, 320 wide below and 400 above
    let [x0, y0, x1, y1] = island.from;
    let [to_x, to_y] = island.to;
    let (width, height) = ((x1 - x0) * island.scale, (y1 - y0) * island.scale);
    let x = (240.0 - to_y - height) - (240.0 - y1) * island.scale;
    let y = (400.0 - to_x - width) - (320.0 - x1) * island.scale;
    // the vertices move rather than the viewport, which would reach past
    // the top screen's buffer for an island moved up from the bottom, and
    // the GPU draws such a viewport wrong; it covers the whole top screen
    let mut vertices = vertices;
    for vertex in &mut vertices {
        let w = vertex.clip[3];
        let below = [(vertex.clip[0] / w * 0.5 + 0.5) * 240.0, (vertex.clip[1] / w * 0.5 + 0.5) * 320.0];
        let above = [x + below[0] * island.scale, y + below[1] * island.scale];
        vertex.clip[0] = (above[0] / 120.0 - 1.0) * w;
        vertex.clip[1] = (above[1] / 200.0 - 1.0) * w;
    }
    let mut registers = onto_top(registers, top, 1.0, 0.0, 0.0);
    registers[REG_VIEWPORT_HEIGHT] = float24(200.0);
    let draw = PortraitDraw { registers, vertices };
    match in_retro {
        true => resources.kiu.retro_draws.push(draw),
        false => resources.kiu.hud.push(draw),
    }
}

/// draws the portraits the bottom screen took last frame into the top
/// screen's buffer, fading in a new one over the one before, and out one the
/// game stopped drawing. whether it drew any. once a frame: under the
/// subtitles when there are any, else just before the top screen goes out.
pub(crate) fn flush_portraits<M: GpuMemory>(memory: &mut M, resources: &mut Resources) -> bool {
    // drawn as the title's own draws are, into the buffer and its twin, the
    // picture in between, whichever draw goes over them
    #[cfg(feature = "vulkan")]
    let pass = resources.hardware.as_mut().filter(|hardware| hardware.twin_cuts).map(|hardware| std::mem::replace(&mut hardware.pass, Pass::Real));
    let drew = draw_portraits(memory, resources);
    #[cfg(feature = "vulkan")]
    if let (Some(pass), Some(hardware)) = (pass, resources.hardware.as_mut()) {
        hardware.pass = pass;
    }
    drew
}

/// how much of a portrait fades in or out each frame, 8 frames in all.
const PORTRAIT_STEP: f32 = 1.0 / 8.0;

/// draws them, see flush_portraits.
fn draw_portraits<M: GpuMemory>(memory: &mut M, resources: &mut Resources) -> bool {
    let step = PORTRAIT_STEP;
    let mut sides = std::mem::take(&mut resources.kiu.sides);
    let mut drew = false;
    for side in &mut sides {
        let incoming = std::mem::take(&mut side.incoming);
        match (incoming.is_empty(), &mut side.shown) {
            // the same picture, where the game has it now
            (false, Some(shown)) if shown.draws.first().map(PortraitDraw::picture) == incoming.first().map(PortraitDraw::picture) => {
                shown.draws = incoming;
                shown.alpha = (shown.alpha + step).min(1.0);
            }
            // a new one, over the one before
            (false, _) => {
                side.leaving = side.shown.take().or(side.leaving.take());
                side.shown = Some(Faded { draws: incoming, alpha: step });
            }
            // the game stopped drawing it
            (true, Some(shown)) => shown.alpha -= step,
            (true, None) => {}
        }
        if let Some(leaving) = &mut side.leaving {
            leaving.alpha -= step;
        }
        for faded in [&mut side.leaving, &mut side.shown] {
            if faded.as_ref().is_some_and(|faded| faded.alpha <= 0.0) {
                *faded = None;
            }
        }
        for faded in [&side.leaving, &side.shown].into_iter().flatten() {
            for draw in &faded.draws {
                draw_faded(memory, resources, draw, faded.alpha);
                drew = true;
            }
        }
    }
    resources.kiu.sides = sides;
    // the retro scene, fading in and out, partly see-through
    resources.kiu.retro = false;
    let incoming = std::mem::take(&mut resources.kiu.retro_draws);
    let mut retro = resources.kiu.retro_shown.take();
    let alpha = retro_alpha(retro.as_ref().map_or(0.0, |shown| shown.alpha), !incoming.is_empty());
    match (incoming.is_empty(), &mut retro) {
        (false, _) => retro = Some(Faded { draws: incoming, alpha }),
        (true, Some(shown)) => shown.alpha = alpha,
        (true, None) => {}
    }
    if let Some(shown) = retro.as_ref().filter(|shown| shown.alpha > 0.0) {
        // its edges, all its draws' together, fading out
        let at = |vertex: &Vertex, axis: usize| vertex.clip[axis] / vertex.clip[3];
        let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
        for vertex in shown.draws.iter().flat_map(|draw| &draw.vertices) {
            for axis in 0..2 {
                low[axis] = low[axis].min(at(vertex, axis));
                high[axis] = high[axis].max(at(vertex, axis));
            }
        }
        let feather = [RETRO_FEATHER / 120.0, RETRO_FEATHER / 200.0];
        for draw in &shown.draws {
            let mut registers = by_vertex_alpha(&draw.registers);
            let topology = registers[REG_PRIMITIVE_CONFIG] >> 8 & 3;
            registers[REG_PRIMITIVE_CONFIG] &= !(3 << 8);
            let mut vertices = draw.vertices.clone();
            for vertex in &mut vertices {
                vertex.color[3] *= shown.alpha;
            }
            let vertices = feathered(topology, &vertices, (low, high), feather);
            rasterize(&registers, memory, resources, Vertices::Shaded(&vertices));
            drew = true;
        }
    }
    resources.kiu.retro_shown = retro.filter(|shown| shown.alpha > 0.0);
    // the battle display as the bottom screen had it, no fading, it is
    // there whenever the game draws it
    let hud = std::mem::take(&mut resources.kiu.hud);
    for mut draw in hud {
        // shaking with the top screen's HUD, as when Pit is hit
        shake(&mut draw.vertices, resources.kiu.shake);
        rasterize(&draw.registers, memory, resources, Vertices::Shaded(&draw.vertices));
        drew = true;
    }
    drew
}

/// draws a portrait with only alpha of it showing over what is there.
fn draw_faded<M: GpuMemory>(memory: &mut M, resources: &mut Resources, draw: &PortraitDraw, alpha: f32) {
    if alpha >= 1.0 {
        rasterize(&draw.registers, memory, resources, Vertices::Shaded(&draw.vertices));
        return;
    }
    let mut registers = draw.registers.clone();
    let mut vertices = draw.vertices.clone();
    if registers[REG_COLOR_OPERATION] & 0x100 == 0 {
        // blends it with the constant alpha, color source * alpha plus the
        // buffer * (1 - alpha), the buffer's alpha kept
        registers[REG_COLOR_OPERATION] |= 0x100;
        registers[REG_BLEND_FUNC] = 12 << 16 | 13 << 20 | 1 << 28;
        registers[REG_BLEND_COLOR] = ((alpha * 255.0).round() as u32) << 24;
    } else {
        // it blends already, by its own alpha as the game draws it
        for vertex in &mut vertices {
            vertex.color[3] *= alpha;
        }
    }
    rasterize(&registers, memory, resources, Vertices::Shaded(&vertices));
}

/// a register's 24-bit float, the inverse of read_float24, for normal
/// values.
fn float24(value: f32) -> u32 {
    if value == 0.0 {
        return 0;
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xFF) - 127 + 63;
    ((bits >> 31) << 23) | (exponent << 16) | ((bits >> 7) & 0xFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::tests::{cover, pixels, quad, rasterize_shaded, sized, target_registers, ConsoleMemory, BLUE, COLOR, RED};

    /// subtitles whose lines are not known are drawn as they are, but on a
    /// widened top screen their text is squeezed as their shadow is, so the
    /// two stay together.
    #[test]
    fn subtitles_not_centered_are_squeezed_with_their_shadow() {
        PORTRAITS.with(|on| on.set(true));
        let mut registers = target_registers();
        sized(&mut registers, 240, 400);
        registers[REG_TEXTURE_CONFIG] = 1;
        registers[TEXTURE_UNIT_BASES[0] + 1] = 512 << 16 | 128;
        let mut resources = Resources::default();
        resources.kiu.widening = 2.0;
        let mut memory = ConsoleMemory::default();
        rasterize_shaded(&registers, &mut memory, &mut resources, &quad((-0.2, 0.2), (-0.25, 0.25), RED));
        PORTRAITS.with(|on| on.set(false));
        assert_eq!(resources.kiu.hud_edges.len(), 2, "the shadow and the text squeezed");
    }

    /// a bottom-screen draw in its battle display's island is taken for the
    /// top screen once, by the title's own draw, not again by the draw in
    /// between, which goes into twins alone; with every twin kept, as before,
    /// it took a second copy.
    #[cfg(feature = "vulkan")]
    #[test]
    fn the_draw_in_between_takes_no_second_copy_of_the_bottom_screen() {
        PORTRAITS.with(|on| on.set(true));
        let mut registers = target_registers();
        sized(&mut registers, 240, 320);
        registers[REG_TEXTURE_CONFIG] = 1;
        // in the island against the bottom screen's top right corner
        let hud = quad((0.6667, 0.9167), (-0.9375, -0.375), RED);
        for twin_cuts in [true, false] {
            let Ok(mut hardware) = hardware::Hardware::new() else { break };
            hardware.twin_cuts = twin_cuts;
            hardware.set_twins(true);
            let mut resources = Resources { hardware: Some(hardware), ..Default::default() };
            resources.kiu.in_play = true;
            resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
            let mut memory = ConsoleMemory::default();
            for pass in [Pass::Twinned, Pass::Twin] {
                resources.hardware.as_mut().unwrap().pass = pass;
                rasterize_shaded(&registers, &mut memory, &mut resources, &hud);
            }
            assert_eq!(resources.kiu.hud.len(), if twin_cuts { 1 } else { 2 }, "with twin cuts {twin_cuts}");
        }
        PORTRAITS.with(|on| on.set(false));
    }

    /// the portraits drawn into the top screen's buffer go into its twin as
    /// well, the picture in between has them too, even when the draw they
    /// go under, the subtitles', is one a draw in between follows.
    #[cfg(feature = "vulkan")]
    #[test]
    fn portraits_go_into_the_picture_in_between_too() {
        let registers = target_registers();
        for twin_cuts in [true, false] {
            let Ok(mut hardware) = hardware::Hardware::new() else { return };
            hardware.twin_cuts = twin_cuts;
            hardware.set_twins(true);
            let mut resources = Resources { hardware: Some(hardware), ..Default::default() };
            let mut memory = ConsoleMemory::default();
            let pass = |resources: &mut Resources, pass| resources.hardware.as_mut().unwrap().pass = pass;
            // a draw that moved gave the buffer a twin
            pass(&mut resources, Pass::Twinned);
            rasterize_shaded(&registers, &mut memory, &mut resources, &cover(-0.5, BLUE));
            pass(&mut resources, Pass::Twin);
            rasterize_shaded(&registers, &mut memory, &mut resources, &cover(-0.5, BLUE));
            // the portraits go in during the title's own draw of the next
            pass(&mut resources, Pass::Twinned);
            resources.kiu.hud.push(PortraitDraw { registers: registers.clone(), vertices: cover(-0.5, RED) });
            let before = resources.hardware.as_ref().unwrap().twin_work(COLOR)[0];
            flush_portraits(&mut memory, &mut resources);
            let drawn = resources.hardware.as_ref().unwrap().twin_work(COLOR)[0] - before;
            assert_eq!(drawn, if twin_cuts { 1 } else { 0 }, "into the twin with twin cuts {twin_cuts}");
        }
    }

    /// on a top screen shown wider than the console's, the HUD's 2D draws
    /// are squeezed back into shape: pieces along an edge toward that edge,
    /// the rest toward their own middle; the 3D scene, what spans the
    /// screen and the console's own shape are left as they are.
    /// squeeze_hud for a draw that is not the title's text.
    fn squeeze_hud_plain(vertices: &[Vertex], factor: f32, edges: &mut Vec<EdgePiece>) -> Option<Vec<Vertex>> {
        squeeze_hud(vertices, factor, false, edges)
    }

    /// text the title writes straight onto the top screen in several draws,
    /// a few letters each, is squeezed toward the screen's middle as one,
    /// so the spaces between the draws shrink with the letters rather than
    /// open up between them.
    #[test]
    fn text_drawn_in_pieces_keeps_its_spacing_on_a_wider_top_screen() {
        let across = |v: &Vertex| 200.0 * (1.0 - v.clip[1]);
        let span = |vertices: &[Vertex]| vertices.iter().map(across).fold((f32::MAX, f32::MIN), |(l, r), x| (l.min(x), r.max(x)));
        let piece = |left: f32, right: f32| quad((-0.9, -0.8), (1.0 - right / 200.0, 1.0 - left / 200.0), RED);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let factor = 1.25;
        let mut edges = Vec::new();
        let (_, first) = span(&squeeze_hud(&piece(100.0, 180.0), factor, true, &mut edges).unwrap());
        let (second, end) = span(&squeeze_hud(&piece(190.0, 300.0), factor, true, &mut edges).unwrap());
        assert!(near(second - first, 10.0 / factor), "the space between them, {first} to {second}");
        assert!(near(end, 200.0 + 100.0 / factor), "toward the screen's middle, ending at {end}");
        // its last piece, short of the right edge, goes with the line too
        let (last, _) = span(&squeeze_hud(&piece(302.0, 353.0), factor, true, &mut edges).unwrap());
        assert!(near(last, 200.0 + 102.0 / factor), "the line's last piece, from {last}");
        // text against an edge still keeps to it
        let (left, _) = span(&squeeze_hud(&piece(14.0, 60.0), factor, true, &mut edges).unwrap());
        assert!(near(left, 14.0 / factor), "text along the left edge, from {left}");
    }

    /// the health bar's inner piece, 90 by 5 pixels at 32 across and 0.8083
    /// down at rest, says how far the title shook its top screen's HUD; any
    /// other piece says nothing.
    #[test]
    fn the_health_bar_says_how_far_the_hud_shook() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let bar = |across: f32, down: f32| quad((0.8083 + down, 0.85 + down), (1.0 - (122.0 + across) / 200.0, 1.0 - (32.0 + across) / 200.0), RED);
        let (across, down) = hud_shake(&bar(0.0, 0.0)).expect("the bar at rest");
        assert!(near(across, 0.0) && near(down, 0.0), "at rest, {across} {down}");
        let (across, down) = hud_shake(&bar(3.5, -0.03)).expect("the bar shaken");
        assert!(near(across, 3.5) && near(down, -0.03), "shaken, {across} {down}");
        assert!(hud_shake(&quad((0.8083, 0.85), (1.0 - 200.0 / 200.0, 1.0 - 32.0 / 200.0), RED)).is_none(), "a longer piece");
        assert!(hud_shake(&quad((-0.9, -0.8), (1.0 - 122.0 / 200.0, 1.0 - 32.0 / 200.0), RED)).is_none(), "a taller piece");
    }

    /// the battle display taken from the bottom screen shakes with the top
    /// screen's HUD: moved across and down as far as the health bar went.
    #[test]
    fn the_islands_shake_with_the_hud() {
        let across = |v: &Vertex| 200.0 * (1.0 - v.clip[1]);
        let mut island = quad((-0.5, -0.4), (1.0 - 60.0 / 200.0, 1.0 - 10.0 / 200.0), RED);
        let before = island.clone();
        shake(&mut island, (3.0, 0.02));
        for (moved, was) in island.iter().zip(&before) {
            assert!((across(moved) - across(was) - 3.0).abs() < 1e-3, "across");
            assert!((moved.clip[0] - was.clip[0] - 0.02).abs() < 1e-5, "down");
        }
    }

    /// a line the title draws in pieces side by side, as a menu's help
    /// line from two strips of pre-drawn text, keeps its spacing: a piece
    /// touching one squeezed before goes toward the same place.
    #[test]
    fn pieces_side_by_side_keep_their_spacing_on_a_wider_top_screen() {
        let across = |v: &Vertex| 200.0 * (1.0 - v.clip[1]);
        let span = |vertices: &[Vertex]| vertices.iter().map(across).fold((f32::MAX, f32::MIN), |(l, r), x| (l.min(x), r.max(x)));
        let piece = |left: f32, right: f32| quad((-0.9, -0.8), (1.0 - right / 200.0, 1.0 - left / 200.0), RED);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let factor = 1.25;
        let mut edges = Vec::new();
        let (_, first) = span(&squeeze_hud_plain(&piece(60.0, 200.0), factor, &mut edges).unwrap());
        let (second, end) = span(&squeeze_hud_plain(&piece(204.0, 320.0), factor, &mut edges).unwrap());
        assert!(near(second - first, 4.0 / factor), "the space between them, {first} to {second}");
        // the end of the line, past a number drawn after it, on the same row
        let (third, _) = span(&squeeze_hud_plain(&piece(340.0, 350.0), factor, &mut edges).unwrap());
        assert!(near(third - end, 20.0 / factor), "the end of the line, {end} to {third}");
    }

    /// what squeeze_hud takes for the title's text: letters from its A4
    /// font, and its 512x128 RGBA8 text buffer, which it shows in more
    /// than one piece; any other picture is not.
    #[test]
    fn text_is_the_font_or_the_text_buffer() {
        let base = TEXTURE_UNIT_BASES[0];
        let texture = |width: u32, height: u32, format: u32| {
            let mut registers = target_registers();
            registers[REG_TEXTURE_CONFIG] |= 1;
            registers[base + 1] = width << 16 | height;
            registers[base + 13] = format;
            registers
        };
        assert!(is_text(&texture(256, 256, 11)), "the font");
        assert!(is_text(&texture(512, 128, 0)), "the text buffer");
        assert!(!is_text(&texture(256, 256, 12)), "a portrait");
        assert!(!is_text(&texture(128, 128, 11)), "another A4 picture");
    }

    #[test]
    fn the_hud_keeps_its_proportions_on_a_wider_top_screen() {
        let across = |v: &Vertex| 200.0 * (1.0 - v.clip[1]);
        let span = |vertices: &[Vertex]| vertices.iter().map(across).fold((f32::MAX, f32::MIN), |(l, r), x| (l.min(x), r.max(x)));
        // a piece from left to right across the top screen's 400
        let piece = |left: f32, right: f32| quad((-0.9, -0.8), (1.0 - right / 200.0, 1.0 - left / 200.0), RED);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let factor = 1.25;
        let mut edges = Vec::new();
        let (left, right) = span(&squeeze_hud_plain(&piece(14.0, 143.0), factor, &mut edges).unwrap());
        assert!(near(left, 14.0 / factor) && near(right, 143.0 / factor), "the health bar, {left} to {right}");
        let (left, right) = span(&squeeze_hud_plain(&piece(312.0, 400.0), factor, &mut edges).unwrap());
        assert!(near(right, 400.0) && near(left, 400.0 - 88.0 / factor), "the score, {left} to {right}");
        let (left, right) = span(&squeeze_hud_plain(&piece(180.0, 240.0), factor, &mut edges).unwrap());
        assert!(near((left + right) / 2.0, 210.0) && near(right - left, 60.0 / factor), "a marker, {left} to {right}");
        // a figure on a badge against the right edge goes with the badge,
        // though it does not reach the edge itself
        let (left, right) = span(&squeeze_hud_plain(&piece(340.0, 350.0), factor, &mut edges).unwrap());
        assert!(near(left, 352.0) && near(right, 360.0), "the figure on the score's badge, {left} to {right}");
        // the same place lower down, touching nothing, keeps its middle
        let lower = quad((0.0, 0.1), (1.0 - 350.0 / 200.0, 1.0 - 340.0 / 200.0), RED);
        let (left, right) = span(&squeeze_hud_plain(&lower, factor, &mut edges).unwrap());
        assert!(near((left + right) / 2.0, 345.0), "a marker lower down, {left} to {right}");
        assert!(squeeze_hud_plain(&piece(0.0, 400.0), factor, &mut edges).is_none(), "across the whole screen");
        let mut scene = piece(100.0, 200.0);
        scene[0].clip[3] = 2.0;
        assert!(squeeze_hud_plain(&scene, factor, &mut edges).is_none(), "the 3D scene");
        assert!(squeeze_hud_plain(&piece(14.0, 143.0), 1.0, &mut edges).is_none(), "the console's shape");
    }

    /// a portrait on one side of the bottom screen goes to the top screen's
    /// corner on that side, scaled by 0.4, with the edge the bottom screen
    /// cuts on the top screen's bottom edge. y runs leftward from the right
    /// edge on both screens, so a low y is the right side.
    #[test]
    fn a_portrait_goes_to_the_bottom_corner_of_its_side() {
        let mut registers = target_registers();
        sized(&mut registers, 240, 320);
        let mut resources = Resources::default();
        let top = [0x1F08_0000 >> 3, 0, 256 | (511 << 12)];
        resources.kiu.top = Some(top);
        // x past the bottom screen's edge, which cuts it at 0, y from 40 to 120
        mirror_portrait(&registers, &mut resources, Vertices::Shaded(&quad((-1.5, 0.0), (-0.75, -0.25), RED)));
        // y from 200 to 280, all of it on the screen, x from 60 to 180
        mirror_portrait(&registers, &mut resources, Vertices::Shaded(&quad((-0.5, 0.5), (0.25, 0.75), RED)));
        let placed = |side: usize| {
            let [draw] = &resources.kiu.sides[side].incoming[..] else { panic!("one draw on side {side}") };
            let xy = draw.registers[REG_VIEWPORT_XY];
            let signed10 = |value: u32| ((value & 0x3FF) << 22) as i32 >> 22;
            assert_eq!(draw.registers[REG_COLOR_BUFFER_ADDRESS], top[0]);
            assert_eq!(draw.registers[REG_FRAMEBUFFER_DIMENSIONS], top[2]);
            assert_eq!(read_float24(&draw.registers, REG_VIEWPORT_WIDTH), 48.0);
            assert_eq!(read_float24(&draw.registers, REG_VIEWPORT_HEIGHT), 64.0);
            assert_eq!(draw.registers[REG_DEPTH_COLOR_MASK] & (1 | 1 << 12), 0, "no depth");
            (signed10(xy), signed10(xy >> 16))
        };
        // the cut edge at 0, the side against its edge: -40 * 0.4
        assert_eq!(placed(0), (0, -16));
        // 60 * 0.4 below, and 400 - 280 * 0.4 along
        assert_eq!(placed(1), (-24, 288));
    }

    /// a portrait as Kid Icarus Uprising draws it, its picture from x -16 to
    /// the bottom screen's top edge at 240, fades out over its last rows on
    /// the top screen, so where the picture is cut does not show as a line.
    #[test]
    fn a_portrait_fades_out_at_the_bottom_screens_top_edge() {
        let mut registers = target_registers();
        sized(&mut registers, 240, 320);
        let mut resources = Resources::default();
        resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
        let quad = quad((-16.0 / 120.0 - 1.0, 1.0), (-0.75, -0.25), RED);
        mirror_portrait(&registers, &mut resources, Vertices::Shaded(&quad));
        let [draw] = &resources.kiu.sides[0].incoming[..] else { panic!("one draw on the right") };
        assert_eq!((draw.registers[REG_PRIMITIVE_CONFIG] >> 8) & 3, 0, "a triangle list");
        assert_eq!(draw.vertices.len() % 3, 0);
        let x = |vertex: &Vertex| (vertex.clip[0] / vertex.clip[3] * 0.5 + 0.5) * 240.0;
        // every corner's alpha follows its rows: whole up to the band, none
        // at the edge, and in between as far as it is into the band
        for vertex in &draw.vertices {
            let expected = ((240.0 - x(vertex)) / PORTRAIT_FADE).clamp(0.0, 1.0);
            assert!((vertex.color[3] - expected).abs() < 1e-4, "alpha {} at x {}", vertex.color[3], x(vertex));
        }
        // the band has corners of its own on its lower edge, so the ramp
        // runs across it alone
        assert!(draw.vertices.iter().any(|vertex| (x(vertex) - (240.0 - PORTRAIT_FADE)).abs() < 1e-3));
        // and all of the picture is still there: the triangles cover what
        // the quad covered
        let area = |vertices: &[Vertex]| -> f32 {
            vertices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| ((t[1].clip[0] - t[0].clip[0]) * (t[2].clip[1] - t[0].clip[1]) - (t[2].clip[0] - t[0].clip[0]) * (t[1].clip[1] - t[0].clip[1])).abs() / 2.0)
                .sum()
        };
        assert!((area(&draw.vertices) - area(&quad)).abs() < 1e-4);
    }

    /// a draw inside the counters' island goes under the score on the top
    /// screen; one over the whole bottom screen, a background, stays there.
    #[test]
    fn the_battle_display_goes_to_its_island_on_the_top_screen() {
        let mut registers = target_registers();
        sized(&mut registers, 240, 320);
        let mut resources = Resources::default();
        resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
        // across 200 to 300 and down 5 to 40 of the bottom screen's 320x240,
        // whose buffer runs up from its bottom edge and left from its right
        mirror_hud(&registers, &mut resources, Vertices::Shaded(&quad((0.6667, 0.9583), (-0.875, -0.25), RED)));
        mirror_hud(&registers, &mut resources, Vertices::Shaded(&quad((-1.0, 1.0), (-1.0, 1.0), RED)));
        let [draw] = &resources.kiu.hud[..] else { panic!("one draw taken, not the background") };
        // drawn over the whole top screen, its vertices moved there
        assert_eq!(draw.registers[REG_VIEWPORT_XY], 0);
        assert_eq!((read_float24(&draw.registers, REG_VIEWPORT_WIDTH), read_float24(&draw.registers, REG_VIEWPORT_HEIGHT)), (120.0, 200.0));
        // the island, 196 to 320 across and 0 to 52 down, lands 325.6 to 400
        // across, against the right edge, and 44 to 75.2 down the top screen,
        // scaled by 0.6: the corner 300 across and 40 down goes to 388 across
        // and 68 down, which in the top screen's buffer is 172 up from its
        // bottom edge and 12 left of its right one
        let screen = to_screen(&draw.vertices[0], (0.0, 0.0, 240.0, 400.0)).unwrap();
        assert!((screen.x - 172.0).abs() < 0.1 && (screen.y - 12.0).abs() < 0.1, "at {}, {}", screen.x, screen.y);
    }

    /// a retro scene on the bottom screen, its 256x128 RGB565 picture across
    /// the screen from 56 to 184 down, goes to the top screen with all that
    /// is drawn over it there, scaled by 0.6 in the middle of it; with no
    /// retro picture that band of the bottom screen stays there.
    #[test]
    fn a_retro_scene_goes_to_the_middle_of_the_top_screen() {
        let mut registers = target_registers();
        sized(&mut registers, 240, 320);
        let base = TEXTURE_UNIT_BASES[0];
        registers[REG_TEXTURE_CONFIG] |= 1;
        let textured = |width: u32, height: u32, format: u32| {
            let mut registers = registers.clone();
            registers[base + 1] = width << 16 | height;
            registers[base + 13] = format;
            registers
        };
        let mut resources = Resources::default();
        resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
        // the band's rows 56 to 184 from the top are 56 to 184 up the buffer
        let band = quad((-0.5333, 0.5333), (-1.0, 1.0), RED);
        let label = quad((0.25, 0.4167), (-0.4375, 0.3125), RED);
        // a draw there that is no retro picture, with none under it, stays below
        mirror_hud(&textured(128, 64, 12), &mut resources, Vertices::Shaded(&label));
        assert!(resources.kiu.hud.is_empty() && resources.kiu.retro_draws.is_empty(), "nothing taken without the retro picture");
        mirror_hud(&textured(256, 128, 3), &mut resources, Vertices::Shaded(&band));
        mirror_hud(&textured(128, 64, 2), &mut resources, Vertices::Shaded(&label));
        assert!(resources.kiu.hud.is_empty(), "apart from the battle display, to fade");
        let [picture, _label] = &resources.kiu.retro_draws[..] else { panic!("the picture and the label taken") };
        // the band, 192 by 76.8 above, from 104 to 296 across and 81.6 to
        // 158.4 down: its corner at the foot on the right, 320 across and
        // 184 down below, lands 296 across and 158.4 down, which in the top
        // screen's buffer is 81.6 up from its bottom edge and 104 left of
        // its right one
        let screen = to_screen(&picture.vertices[0], (0.0, 0.0, 240.0, 400.0)).unwrap();
        assert!((screen.x - 81.6).abs() < 0.1 && (screen.y - 104.0).abs() < 0.1, "at {}, {}", screen.x, screen.y);
        // Hewdraw's is a 64x32 RGBA5551 sprite alone in the band, 112 to 208
        // across and 60 to 184 down, which goes up the same way
        let mut resources = Resources::default();
        resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
        let sprite = quad((56.0 / 120.0 - 1.0, 180.0 / 120.0 - 1.0), (112.0 / 160.0 - 1.0, 208.0 / 160.0 - 1.0), RED);
        mirror_hud(&textured(64, 32, 2), &mut resources, Vertices::Shaded(&sprite));
        assert_eq!(resources.kiu.retro_draws.len(), 1, "Hewdraw's sprite taken");
        // any RGB565 or RGBA5551 picture there is one, the NES game's look;
        // the battle display's ETC1 and RGBA4 pictures are not
        for (format, retro) in [(3, true), (2, true), (12, false), (4, false)] {
            let mut resources = Resources::default();
            resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
            mirror_hud(&textured(32, 32, format), &mut resources, Vertices::Shaded(&sprite));
            assert_eq!(resources.kiu.retro_draws.len() == 1, retro, "format {format}");
        }
        // a small one of those, as the battle display draws while Pit
        // fights, 46 by 39 across 193 to 239 and down 82 to 121, is no
        // retro scene
        let mut resources = Resources::default();
        resources.kiu.top = Some([0x1F08_0000 >> 3, 0, 256 | (511 << 12)]);
        let small = quad(((240.0 - 121.0) / 120.0 - 1.0, (240.0 - 82.0) / 120.0 - 1.0), ((320.0 - 239.0) / 160.0 - 1.0, (320.0 - 193.0) / 160.0 - 1.0), RED);
        mirror_hud(&textured(64, 64, 3), &mut resources, Vertices::Shaded(&small));
        assert!(resources.kiu.retro_draws.is_empty() && !resources.kiu.retro, "a small picture");
    }

    /// the retro scene's edges fade out: a draw over its bounds is cut a
    /// feather's width in from each edge, showing none at the edges and
    /// all of it from the feather in.
    #[test]
    fn a_retro_scene_fades_out_at_its_edges() {
        let whole = quad((-0.5, 0.5), (-0.5, 0.5), RED);
        let bounds = ([-0.5, -0.5], [0.5, 0.5]);
        let cut = feathered(0, &whole, bounds, [0.1, 0.1]);
        assert_eq!(cut.len() % 3, 0, "a list of triangles");
        assert!(cut.len() > whole.len(), "cut along the feathers");
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        for vertex in &cut {
            let [x, y] = [vertex.clip[0], vertex.clip[1]];
            let edge = near(x.abs(), 0.5) || near(y.abs(), 0.5);
            let inside = x.abs() <= 0.4 + 1e-4 && y.abs() <= 0.4 + 1e-4;
            if edge {
                assert!(near(vertex.color[3], 0.0), "none at the edge, {x} {y} {}", vertex.color[3]);
            } else if inside {
                assert!(near(vertex.color[3], RED[3]), "all inside, {x} {y} {}", vertex.color[3]);
            }
        }
        // the same area, cut or not
        let area = |triangles: &[Vertex]| -> f32 {
            triangles.chunks_exact(3).map(|t| ((t[1].clip[0] - t[0].clip[0]) * (t[2].clip[1] - t[0].clip[1]) - (t[2].clip[0] - t[0].clip[0]) * (t[1].clip[1] - t[0].clip[1])).abs() / 2.0).sum()
        };
        assert!(near(area(&cut), 1.0), "area {}", area(&cut));
    }

    /// a retro scene's draws blend by the alpha their vertices carry, the
    /// combiner's last stage taking it in, the colours they make kept.
    #[test]
    fn a_retro_scene_blends_by_its_vertices_alpha() {
        let registers = target_registers();
        let blended = by_vertex_alpha(&registers);
        let last = TEV_STAGES[5];
        assert_eq!(blended[last] & 0xFFFF, registers[last] & 0xFFFF, "its colours as they were");
        assert_eq!((blended[last] >> 16) & 0xFF, 0x0F, "the previous stage's alpha times the vertex's");
        assert_eq!((blended[last + 2] >> 16) & 0xF, 1, "multiplied");
        assert_ne!(blended[REG_COLOR_OPERATION] & 0x100, 0, "blending");
        assert_eq!(blended[REG_BLEND_FUNC], 6 << 16 | 7 << 20 | 1 << 28, "by the source's alpha, the buffer's alpha kept");
        assert_eq!(blended[REG_ALPHA_TEST] & 1, 0, "no alpha test to cut the faded edges off");
        // the game's own retro draws blend by their vertices' alpha already,
        // the texture's times the vertex's: they are left as they are
        let mut blending = registers.clone();
        blending[REG_COLOR_OPERATION] |= 0x100;
        blending[REG_BLEND_FUNC] = 0x0176_0000;
        assert_eq!(by_vertex_alpha(&blending), blending, "a draw that blends already");
    }

    /// a retro scene fades in over a few frames while the game draws it, up
    /// to partly see-through, and out again over as many once it stops.
    #[test]
    fn a_retro_scene_fades_in_and_out() {
        let mut alpha = 0.0;
        let mut frames = 0;
        while alpha < RETRO_ALPHA {
            alpha = retro_alpha(alpha, true);
            frames += 1;
        }
        assert_eq!(alpha, RETRO_ALPHA, "it stops there");
        assert!((5..=10).contains(&frames), "in over {frames} frames");
        const { assert!(RETRO_ALPHA < 1.0 && RETRO_ALPHA > 0.5, "partly see-through") };
        assert_eq!(retro_alpha(alpha, true), RETRO_ALPHA, "and stays");
        let mut frames = 0;
        while alpha > 0.0 {
            alpha = retro_alpha(alpha, false);
            frames += 1;
        }
        assert!((5..=10).contains(&frames), "out over {frames} frames");
    }

    /// a vertex at x, y of a viewport w by h, with texture coordinates u, v.
    fn at(x: f32, y: f32, (w, h): (f32, f32), [u, v]: [f32; 2]) -> Vertex {
        let clip = [x / w * 2.0 - 1.0, y / h * 2.0 - 1.0, -0.5, 1.0];
        Vertex { clip, color: RED, texcoords: [[u, v], [0.0; 2], [0.0; 2]], quaternion: [0.0, 0.0, 0.0, 1.0], view: [0.0; 3], texcoord0_w: 1.0 }
    }

    /// a quad from x0, y0 to x1, y1 of a viewport, as two triangles.
    fn rect(x0: f32, y0: f32, x1: f32, y1: f32, size: (f32, f32)) -> Vec<Vertex> {
        [(x0, y0), (x1, y0), (x1, y1), (x0, y0), (x1, y1), (x0, y1)].into_iter().map(|(x, y)| at(x, y, size, [0.0; 2])).collect()
    }

    /// letters written into the text buffer make its lines, row bands with
    /// how far across they go, and the clearing before new text forgets them.
    #[test]
    fn the_text_buffer_lines_follow_the_letters_written() {
        let mut registers = target_registers();
        sized(&mut registers, 512, 128);
        let base = TEXTURE_UNIT_BASES[0];
        registers[REG_TEXTURE_CONFIG] = 1;
        registers[base + 13] = 11; // A4, the font
        let mut resources = Resources::default();
        let size = (512.0, 128.0);
        // two batches of letters, the second on both lines
        let mut letters = rect(17.0, 35.0, 108.0, 50.0, size);
        letters.extend(rect(17.0, 19.0, 60.0, 34.0, size));
        note_text(&registers, &mut resources, Vertices::Shaded(&letters));
        note_text(&registers, &mut resources, Vertices::Shaded(&rect(113.0, 35.0, 215.0, 50.0, size)));
        let mut lines = resources.kiu.text_lines.clone();
        lines.sort_by(|a, b| a.bottom.total_cmp(&b.bottom));
        assert_eq!(
            lines,
            [
                TextLine { bottom: 19.0, top: 34.0, left: 17.0, right: 60.0 },
                TextLine { bottom: 35.0, top: 50.0, left: 17.0, right: 215.0 }
            ]
        );
        // the clearing, no font, over the text
        registers[base + 13] = 0;
        note_text(&registers, &mut resources, Vertices::Shaded(&rect(3.0, 15.0, 287.0, 58.0, size)));
        assert!(resources.kiu.text_lines.is_empty());
    }

    /// each line of the subtitles slides so its middle is the top screen's
    /// middle, the quads cut into one slice per line. the quad is the one
    /// Kid Icarus Uprising draws: across 97.6 to 361 of the top screen's
    /// buffer, its text buffer's u 0.522 to 0.008 that way.
    #[test]
    fn each_subtitle_line_goes_to_the_middle_of_the_screen() {
        let size = (240.0, 400.0);
        let corner = |x: f32, y: f32, u: f32, v: f32| at(x, y, size, [u, v]);
        let (a, b, c, d) = (corner(56.0, 361.0, 0.008, 0.445), corner(56.0, 97.6, 0.522, 0.445), corner(15.1, 361.0, 0.008, 0.126), corner(15.1, 97.6, 0.522, 0.126));
        let quad = [a, b, c, b, d, c];
        let lines = [TextLine { bottom: 35.0, top: 50.0, left: 17.0, right: 215.0 }, TextLine { bottom: 19.0, top: 34.0, left: 17.0, right: 183.0 }];
        let centered = centered_subtitle(&quad, &lines).expect("centered");
        assert_eq!(centered.len(), 12, "two slices of the quad");
        for (slice, line) in centered.chunks_exact(6).zip([lines[1], lines[0]]) {
            // where the line's middle is now, between the slice's ends in u
            let low_u = slice.iter().find(|v| (v.texcoords[0][0] - 0.008).abs() < 1e-4).unwrap();
            let high_u = slice.iter().find(|v| (v.texcoords[0][0] - 0.522).abs() < 1e-4).unwrap();
            let middle = ((line.left + line.right) / 2.0 / 512.0 - 0.008) / (0.522 - 0.008);
            let y = low_u.clip[1] + (high_u.clip[1] - low_u.clip[1]) * middle;
            assert!(y.abs() < 1e-4, "line {line:?} at {y}");
            // and it covers its own rows of the text buffer
            let vs: Vec<f32> = slice.iter().map(|v| v.texcoords[0][1] * 128.0).collect();
            let (low_v, high_v) = (vs.iter().copied().fold(f32::MAX, f32::min), vs.iter().copied().fold(f32::MIN, f32::max));
            assert!(low_v <= line.bottom && high_v >= line.top, "{line:?} in {low_v}..{high_v}");
        }
        // no lines known: the subtitles are left as the title drew them
        assert!(centered_subtitle(&quad, &[]).is_none());
    }

    /// a portrait fades in over 8 frames, the next picture fades in over it
    /// while it fades out, and one the game stops drawing fades out.
    #[test]
    fn a_portrait_fades_in_over_and_out() {
        let mut registers = target_registers();
        // as Kid Icarus Uprising blends its portraits, by their alpha
        registers[REG_COLOR_OPERATION] = 0x00E4_0100;
        registers[REG_BLEND_FUNC] = 0x0176_0000;
        let mut memory = ConsoleMemory::default();
        let mut resources = Resources::default();
        let draw = |picture: u32| {
            let mut registers = registers.clone();
            registers[TEXTURE_UNIT_BASES[0] + 4] = picture;
            PortraitDraw { registers, vertices: cover(-0.5, RED) }
        };
        let alphas = |resources: &Resources| {
            let side = &resources.kiu.sides[0];
            (side.shown.as_ref().map(|faded| faded.alpha), side.leaving.as_ref().map(|faded| faded.alpha))
        };

        resources.kiu.sides[0].incoming.push(draw(1));
        assert!(flush_portraits(&mut memory, &mut resources));
        assert_eq!(alphas(&resources), (Some(0.125), None));
        let red = pixels(&mut memory)[0][0];
        assert!((28..=36).contains(&red), "an eighth of red over black, not {red}");

        for _ in 0..7 {
            resources.kiu.sides[0].incoming.push(draw(1));
            flush_portraits(&mut memory, &mut resources);
        }
        assert_eq!(alphas(&resources), (Some(1.0), None));

        resources.kiu.sides[0].incoming.push(draw(2));
        flush_portraits(&mut memory, &mut resources);
        assert_eq!(alphas(&resources), (Some(0.125), Some(0.875)), "the new one over the old");

        // the game stops: the new one had an eighth to lose, the old one 7
        assert!(flush_portraits(&mut memory, &mut resources));
        assert_eq!(alphas(&resources), (None, Some(0.75)));
        for _ in 0..5 {
            assert!(flush_portraits(&mut memory, &mut resources));
        }
        assert!(!flush_portraits(&mut memory, &mut resources), "the last step leaves nothing to draw");
        assert_eq!(alphas(&resources), (None, None));
    }
}
