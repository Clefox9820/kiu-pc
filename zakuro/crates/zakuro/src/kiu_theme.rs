//! Kid Icarus Uprising's look for everything the emulator shows: ink on
//! parchment, a red banner for what matters most, heavy titles over a
//! magenta to cyan flourish and black vines in the corners, as the game's own
//! menus are drawn.

use egui::{pos2, vec2, Color32, CornerRadius, FontId, Sense, Stroke, Ui, Vec2};

/// Kid Icarus' menu colours: black ink on parchment, a red banner for what
/// matters most and a magenta to cyan flourish under titles.
pub const PAPER: Color32 = Color32::from_rgb(243, 235, 221);
pub const PAPER_LINE: Color32 = Color32::from_rgb(234, 224, 207);
pub const PAPER_SHADOW: Color32 = Color32::from_rgb(196, 183, 162);
pub const INK: Color32 = Color32::from_rgb(26, 20, 18);
pub const RED: Color32 = Color32::from_rgb(222, 32, 36);
pub const ORANGE: Color32 = Color32::from_rgb(250, 164, 40);
const MAGENTA: Color32 = Color32::from_rgb(232, 64, 196);
const BLUE: Color32 = Color32::from_rgb(84, 110, 240);
const CYAN: Color32 = Color32::from_rgb(64, 216, 232);
pub const SHADOW: Color32 = Color32::from_black_alpha(110);
/// how far a window's or a panel's hard shadow falls, right and down.
pub const SHADOW_OFFSET: [i8; 2] = [3, 4];
/// the faint ink of hints and second lines, readable on the parchment.
const FADED_INK: Color32 = Color32::from_rgb(98, 86, 74);
/// the banner red darkened to read as text on the parchment.
const DARK_RED: Color32 = Color32::from_rgb(176, 20, 24);

/// Kid Icarus' parchment for everything the emulator shows, its windows,
/// menus and notices: ink on paper, red when touched, whatever theme the
/// system asks for. one theme set once keeps the emulator's own windows as
/// Zakuro wrote them.
pub fn parchment(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let visuals = &mut style.visuals;
        *visuals = egui::Visuals::light();
        paper(visuals);
        // the windows' own colours, links and warnings stay theirs
        visuals.override_text_color = None;
        visuals.weak_text_color = Some(FADED_INK);
        visuals.window_fill = PAPER;
        visuals.panel_fill = PAPER;
        visuals.faint_bg_color = PAPER_LINE;
        visuals.code_bg_color = PAPER_LINE;
        // the title bar of the window in front, and an open list's button
        visuals.widgets.open.weak_bg_fill = PAPER_LINE;
        visuals.window_stroke = Stroke::new(1.5, INK);
        visuals.window_corner_radius = CornerRadius::same(8);
        visuals.menu_corner_radius = CornerRadius::same(6);
        visuals.window_shadow = egui::Shadow { offset: SHADOW_OFFSET, blur: 0, spread: 0, color: SHADOW };
        visuals.popup_shadow = egui::Shadow { offset: [2, 3], blur: 0, spread: 0, color: SHADOW };
        visuals.hyperlink_color = DARK_RED;
        visuals.error_fg_color = DARK_RED;
        visuals.warn_fg_color = Color32::from_rgb(150, 72, 0);
    });
}

/// the parchment's look for the widgets: ink on white, red when touched.
fn paper(visuals: &mut egui::Visuals) {
    visuals.override_text_color = Some(INK);
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.slider_trailing_fill = true;
    visuals.selection.bg_fill = RED;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    let widgets = &mut visuals.widgets;
    for (widget, edge) in [(&mut widgets.inactive, INK), (&mut widgets.hovered, RED), (&mut widgets.active, RED), (&mut widgets.open, RED)] {
        widget.bg_fill = Color32::WHITE;
        widget.weak_bg_fill = Color32::WHITE;
        widget.bg_stroke = Stroke::new(1.2, edge);
        widget.fg_stroke = Stroke::new(1.5, INK);
    }
    widgets.hovered.weak_bg_fill = PAPER_LINE;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, INK);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK);
}

/// a window's title as the game's menus draw it, across the window's width.
pub fn title(ui: &mut Ui, text: &str) {
    let (place, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    heavy_title(ui.painter(), text, place.center().x, place.min.y, 1.0);
}

/// `a` turned toward `b` by `t`, from 0 to 1.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// the flourish's colour at `t` across it: magenta, then blue, then cyan.
fn rainbow(t: f32) -> Color32 {
    if t < 0.5 { mix(MAGENTA, BLUE, t * 2.0) } else { mix(BLUE, CYAN, t * 2.0 - 1.0) }
}

/// a leaf: a lozenge from `base`, `length` long toward `dir`.
fn leaf(painter: &egui::Painter, base: egui::Pos2, dir: Vec2, length: f32) {
    let side = Vec2::new(-dir.y, dir.x) * length * 0.28;
    let middle = base + dir * length * 0.5;
    painter.add(egui::Shape::convex_polygon(vec![base, middle + side, base + dir * length, middle - side], INK, Stroke::NONE));
}

/// a black vine curling out of a corner, as on the game's parchment menus:
/// it grows from `at` along `along` and curls toward `inward`.
pub fn vine(painter: &egui::Painter, at: egui::Pos2, along: Vec2, inward: Vec2, size: f32) {
    // turning from `along` to `inward` is clockwise on screen when this is 1
    let turn = along.x * inward.y - along.y * inward.x;
    let width = size * 0.06;
    // a spiral round `centre`, starting `radius` out toward `from` and
    // winding in by `turn`; gives where it starts
    let spiral = |centre: egui::Pos2, from: Vec2, radius: f32, turn: f32, width: f32| {
        let start = from.y.atan2(from.x);
        let points: Vec<_> = (0..=40)
            .map(|i| {
                let t = i as f32 / 40.0;
                let angle = start + turn * t * std::f32::consts::TAU * 1.3;
                centre + Vec2::new(angle.cos(), angle.sin()) * radius * (1.0 - 0.75 * t)
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(width, INK)));
        centre + from * radius
    };
    let stem = |from: egui::Pos2, to: egui::Pos2, bow: Vec2, width: f32| {
        let points = [from, from + (to - from) * 0.4 + bow, to - (to - from) * 0.2 + bow, to];
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, Stroke::new(width, INK)));
    };
    // the long arm along the edge, ending in a curl that turns inward
    let curl = spiral(at + along * size * 0.95 + inward * size * 0.26, -inward, size * 0.26, turn, width);
    stem(at, curl, inward * size * 0.06, width);
    // a shorter arm down the side, curling the other way
    let side = spiral(at + inward * size * 0.7 + along * size * 0.2, -along, size * 0.18, -turn, width * 0.8);
    stem(at, side, along * size * 0.05, width * 0.8);
    for (k, a, i) in [(0.35, 0.6, 0.8), (0.6, 0.9, -0.2), (0.2, 0.3, 1.0)] {
        leaf(painter, at + along * size * k + inward * size * 0.03, (along * a + inward * i).normalized(), size * 0.26);
    }
    leaf(painter, at + inward * size * 0.35, (along * 0.9 + inward * 0.4).normalized(), size * 0.22);
}

/// the two vines in a panel's top corners, `inset` in from its edges.
pub fn corner_vines(painter: &egui::Painter, panel: egui::Rect, inset: f32, size: f32) {
    vine(painter, panel.min + vec2(inset, inset), vec2(1.0, 0.0), vec2(0.0, 1.0), size);
    vine(painter, pos2(panel.max.x - inset, panel.min.y + inset), vec2(-1.0, 0.0), vec2(0.0, 1.0), size);
}

/// a small heart, the game's coin, centred on `at`.
fn heart(painter: &egui::Painter, at: egui::Pos2, size: f32) {
    painter.circle_filled(pos2(at.x - size * 0.5, at.y - size * 0.25), size * 0.56, RED);
    painter.circle_filled(pos2(at.x + size * 0.5, at.y - size * 0.25), size * 0.56, RED);
    let points = vec![pos2(at.x - size * 1.04, at.y - size * 0.1), pos2(at.x + size * 1.04, at.y - size * 0.1), pos2(at.x, at.y + size * 1.1)];
    painter.add(egui::Shape::convex_polygon(points, RED, Stroke::NONE));
}

/// a title as the game's menus draw it, centred on `middle` from `top`: heavy
/// ink with a white edge, over a ribbon thick in the middle and thin at the
/// ends, magenta to cyan, curling at both ends round a heart. gives the
/// ribbon's height.
pub fn heavy_title(painter: &egui::Painter, text: &str, middle: f32, top: f32, scale: f32) -> f32 {
    let unit = |v: f32| v * scale;
    let layout = |color| painter.layout_no_wrap(text.to_owned(), FontId::proportional(unit(19.0)), color);
    let (title, edge) = (layout(INK), layout(Color32::WHITE));
    // a white edge, then the ink twice a hair apart
    let title_at = pos2(middle - title.size().x / 2.0, top);
    for (dx, dy) in [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.6, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.6, 1.0)] {
        painter.galley(title_at + vec2(unit(dx * 1.4), unit(dy * 1.4)), edge.clone(), Color32::WHITE);
    }
    painter.galley(title_at, title.clone(), INK);
    painter.galley(title_at + vec2(unit(0.7), 0.0), title.clone(), INK);
    let y = top + title.size().y + unit(7.0);

    let half = title.size().x / 2.0 + unit(26.0);
    let steps = 48;
    for step in 0..steps {
        let (t0, t1) = (step as f32 / steps as f32, (step + 1) as f32 / steps as f32);
        let (x0, x1) = (middle - half + 2.0 * half * t0, middle - half + 2.0 * half * t1);
        // a gap for the heart
        if ((x0 + x1) / 2.0 - middle).abs() < unit(9.0) {
            continue;
        }
        let swell = 1.0 - (t0 + t1 - 1.0).abs();
        painter.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(unit(0.8 + 2.4 * swell), rainbow((t0 + t1) / 2.0)));
    }
    for (end, side, color) in [(middle - half, -1.0, MAGENTA), (middle + half, 1.0, CYAN)] {
        let up = [pos2(end, y), pos2(end + side * unit(9.0), y), pos2(end + side * unit(9.0), y - unit(7.0)), pos2(end + side * unit(3.0), y - unit(6.0))];
        let down = [pos2(end, y), pos2(end + side * unit(6.0), y), pos2(end + side * unit(6.0), y + unit(5.0)), pos2(end + side * unit(2.0), y + unit(4.0))];
        for points in [up, down] {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, Stroke::new(unit(1.2), color)));
        }
    }
    heart(painter, pos2(middle, y), unit(4.5));
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::Menus;

    /// how far apart two colours' lightness is, 1 to 21, as WCAG measures it.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lightness = |c: Color32| {
            let channel = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (a, b) = (lightness(a), lightness(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_window_is_parchment_with_ink_that_reads_on_it() {
        // whichever theme the system asks for
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            parchment(&ctx);
            let visuals = ctx.global_style().visuals.clone();
            assert_eq!((visuals.window_fill, visuals.panel_fill), (PAPER, PAPER), "{theme:?}");
            let widgets = &visuals.widgets;
            for (name, widget) in [("still", widgets.noninteractive), ("button", widgets.inactive), ("hovered", widgets.hovered), ("pressed", widgets.active), ("open", widgets.open)] {
                assert!(contrast(widget.fg_stroke.color, widget.bg_fill) >= 4.5, "{name} text on its fill, {theme:?}");
                assert!(contrast(widget.fg_stroke.color, widget.weak_bg_fill) >= 4.5, "{name} text on its weak fill, {theme:?}");
            }
            assert!(contrast(visuals.weak_text_color(), PAPER) >= 4.5, "faint text, {theme:?}");
            assert!(contrast(visuals.selection.stroke.color, visuals.selection.bg_fill) >= 4.5, "chosen text, {theme:?}");
            for (name, colour) in [("warning", visuals.warn_fg_color), ("error", visuals.error_fg_color), ("link", visuals.hyperlink_color)] {
                assert!(contrast(colour, PAPER) >= 4.5, "{name}, {theme:?}");
            }
        }
    }

    /// one frame of the pause menu on a 1200x720 window, with this input, and
    /// what it drew.
    fn frame(ctx: &egui::Context, menus: &mut Menus, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1200.0, 720.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        ctx.run_ui(input, |ui| {
            menus.game(ui, "Kid Icarus: Uprising", None, true, false, &[], None);
        })
        .shapes
    }

    /// the pause menu after it settles, and what it drew last.
    fn pause_menu(menus: &mut Menus) -> (egui::Context, Vec<egui::epaint::ClippedShape>) {
        let ctx = egui::Context::default();
        parchment(&ctx);
        // a window's first pass only measures it
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&ctx, menus, Vec::new());
        }
        (ctx, shapes)
    }

    /// the pause menu's wide buttons, from the top.
    fn wide_buttons(ctx: &egui::Context) -> Vec<egui::WidgetRect> {
        let mut buttons: Vec<egui::WidgetRect> = ctx.viewport(|viewport| {
            viewport.prev_pass.widgets.layers().flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| (widget.rect.width() - 200.0).abs() < 0.5)
                .copied()
                .collect()
        });
        buttons.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));
        buttons
    }

    #[test]
    fn the_pause_menu_is_centred_under_its_title() {
        let mut menus = Menus::default();
        menus.menu_open = true;
        let (ctx, _) = pause_menu(&mut menus);
        let buttons = wide_buttons(&ctx);
        assert!(!buttons.is_empty());
        let window = ctx.memory(|memory| memory.area_rect(buttons[0].layer_id.id)).unwrap();
        for button in buttons.iter().map(|widget| widget.rect) {
            let (left, right) = (button.min.x - window.min.x, window.max.x - button.max.x);
            assert!((left - right).abs() < 4.0, "{button:?} in {window:?}: {left} left, {right} right");
        }
    }

    /// every line of text a shape draws, inside groups too.
    fn texts(shape: &egui::Shape, into: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => into.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, into)),
            _ => {}
        }
    }

    fn shown(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        let mut shown = Vec::new();
        shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut shown));
        shown
    }

    #[test]
    fn the_pause_menu_has_one_way_into_the_settings() {
        let mut menus = Menus::default();
        menus.menu_open = true;
        let (_, shapes) = pause_menu(&mut menus);
        let shown = shown(&shapes);
        assert!(shown.iter().any(|text| text == "Options"), "{shown:?}");
        assert!(!shown.iter().any(|text| text == "Settings"), "one way in, through the options: {shown:?}");
    }

    #[test]
    fn options_in_the_pause_menu_open_the_settings_in_its_place() {
        let mut menus = Menus::default();
        menus.menu_open = true;
        let (ctx, _) = pause_menu(&mut menus);
        // under the title, Resume, then Options, each a button inside the
        // space it was given, both the same rectangle
        let mut buttons: Vec<_> = wide_buttons(&ctx).into_iter().map(|widget| widget.rect).filter(|rect| (rect.height() - 28.0).abs() < 0.5).collect();
        buttons.dedup();
        let options = buttons[1].center();
        let click = |pressed| egui::Event::PointerButton { pos: options, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&ctx, &mut menus, vec![egui::Event::PointerMoved(options), click(true)]);
        frame(&ctx, &mut menus, vec![click(false)]);
        assert!(menus.settings_open, "the native settings open");
        assert!(menus.menu_open, "and close back to the pause menu");
        let shapes = frame(&ctx, &mut menus, Vec::new());
        assert!(!shown(&shapes).iter().any(|text| text == "Resume"), "the settings take the menu's place");
    }

    #[test]
    fn the_settings_hold_what_kid_icarus_adds() {
        let ctx = egui::Context::default();
        parchment(&ctx);
        let mut menus = Menus::default();
        menus.settings_open = true;
        let mut settings = crate::settings::Settings::default();
        // tall enough for the whole window without scrolling
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1200.0, 4000.0));
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            shapes = ctx.run_ui(input, |ui| {
                assert!(menus.settings(ui.ctx(), &mut settings).is_empty(), "nothing changed, nothing to do");
            })
            .shapes;
        }
        let shown = shown(&shapes);
        for row in ["Invert mouse aim up and down", "Take the mouse back", "Pause in the background", "Discord status", "Fullscreen", "Sharp HUD, menus and portraits"] {
            assert!(shown.iter().any(|text| text == row), "{row} in {shown:?}");
        }
    }
}
