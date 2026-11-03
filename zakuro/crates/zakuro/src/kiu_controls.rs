//! the keyboard and mouse controls of Kid Icarus Uprising, whose port this
//! is, drawn beside the screens as the game's own menus are.

use egui::{Color32, Vec2};

use crate::kiu_theme::{corner_vines, heavy_title, INK, ORANGE, PAPER, PAPER_LINE, PAPER_SHADOW, RED, SHADOW, SHADOW_OFFSET};
use crate::menus::key_name;
use crate::settings::Keys;

/// a line of the controls: the keys, each drawn as a cap, and what they do.
struct ControlRow {
    keys: Vec<String>,
    does: &'static str,
}

/// the keyboard and mouse controls in sections, for Kid Icarus Uprising,
/// whose port this is, from the keys bound now.
fn controls_sections(k: &Keys) -> Vec<(&'static str, Vec<ControlRow>)> {
    let row = |keys: Vec<String>, does| ControlRow { keys, does };
    let n = key_name;
    let text = |name: &str| name.to_owned();
    vec![
        ("MOVE", vec![
            row(vec![n(k.circle_up), n(k.circle_left), n(k.circle_down), n(k.circle_right)], "Move, twice to dash"),
            row(vec![text("Shift")], "Walk"),
            row(vec![n(k.r)], "Reset camera"),
        ]),
        ("ATTACK", vec![row(vec![n(k.l), text("Left click")], "Fire, hold for more")]),
        ("POWERS", vec![
            row(vec![n(k.left), n(k.right), text("Wheel")], "Pick a power"),
            row(vec![n(k.down), text("Middle click")], "Use it"),
        ]),
        ("AIM", vec![
            row(vec![text("1"), text("2"), text("3")], "Off, air, land"),
            row(vec![text("Mouse")], "Turns the camera on land"),
            row(vec![text("Right click")], "Hold to drag the stylus on land"),
        ]),
        ("MENUS", vec![
            row(vec![n(k.a), n(k.b)], "A, B"),
            row(vec![n(k.x), n(k.y)], "X, Y"),
            row(vec![n(k.start), n(k.select)], "Start, Select"),
            row(vec![n(k.up)], "D-pad up"),
            row(vec![text("Esc")], "Emulator menu"),
        ]),
    ]
}

/// lays the controls panel out at a scale, its sections in columns, painting
/// it at `at` when given, and says how big it is. it is drawn as the game's
/// own menus are: ink on parchment with vines in the corners, a heavy title
/// over a rainbow flourish, headings on slanted black plates, keys as white
/// pills and a black strip of advice at the foot.
fn controls_panel(
    painter: &egui::Painter,
    columns: &[&[(&str, Vec<ControlRow>)]],
    scale: f32,
    at: Option<egui::Pos2>,
    active: &str,
) -> Vec2 {
    use egui::{pos2, vec2, CornerRadius, FontId, Rect, Shape, Stroke, StrokeKind};
    const TIP: &str = "The aim in use is lit in red";
    let unit = |v: f32| v * scale;
    let (pad, cap_h, cap_pad, gap, row_gap, section_gap, column_gap) =
        (unit(14.0), unit(19.0), unit(7.0), unit(4.0), unit(5.0), unit(10.0), unit(22.0));
    let (plate_pad, slant, footer_h) = (unit(7.0), unit(4.0), unit(20.0));
    let layout = |text: &str, size: f32, color| painter.layout_no_wrap(text.to_owned(), FontId::proportional(unit(size)), color);
    let title = layout("CONTROLS", 19.0, INK);
    let tip = layout(TIP, 11.0, Color32::WHITE);
    let plate = |name: &str| layout(name, 10.0, Color32::WHITE).size() + vec2(2.0 * plate_pad + slant, unit(4.0));
    let plate_h = plate("MOVE").y;
    let row_width = |row: &ControlRow| {
        let caps: f32 = row.keys.iter().map(|key| layout(key, 11.0, INK).size().x + 2.0 * cap_pad + gap).sum();
        caps + unit(6.0) + layout(row.does, 12.0, INK).size().x
    };
    let column_size = |sections: &[(&str, Vec<ControlRow>)]| {
        let rows_width = sections.iter().flat_map(|(_, rows)| rows).map(row_width).fold(0.0, f32::max);
        let width = sections.iter().map(|(name, _)| plate(name).x).fold(rows_width, f32::max);
        let rows = sections.iter().map(|(_, rows)| rows.len()).sum::<usize>() as f32;
        let height = sections.len() as f32 * (plate_h + unit(5.0))
            + rows * (cap_h + row_gap)
            + (sections.len() as f32 - 1.0).max(0.0) * section_gap;
        vec2(width, height)
    };
    let sizes: Vec<Vec2> = columns.iter().map(|column| column_size(column)).collect();
    let body = vec2(
        sizes.iter().map(|size| size.x).sum::<f32>() + column_gap * (sizes.len() as f32 - 1.0),
        sizes.iter().map(|size| size.y).fold(0.0, f32::max),
    );
    // room beside the title for the vines in the corners
    let inner_width = body.x.max(title.size().x + unit(100.0)).max(tip.size().x + unit(16.0));
    let width = inner_width + 2.0 * pad;
    let height = pad + title.size().y + unit(17.0) + body.y + unit(10.0) + footer_h;
    let Some(at) = at else { return vec2(width, height) };

    let panel = Rect::from_min_size(at, vec2(width, height));
    let corner = unit(8.0);
    let [shadow_x, shadow_y] = SHADOW_OFFSET.map(|offset| unit(offset as f32));
    painter.rect_filled(panel.translate(vec2(shadow_x, shadow_y)), corner, SHADOW);
    painter.rect_filled(panel, corner, PAPER);
    // the faint ruled lines of the game's parchment
    let mut line = panel.min.y + corner;
    while line < panel.max.y - footer_h {
        painter.line_segment([pos2(panel.min.x + unit(2.0), line), pos2(panel.max.x - unit(2.0), line)], Stroke::new(unit(1.0), PAPER_LINE));
        line += unit(5.0);
    }
    corner_vines(painter, panel, unit(7.0), unit(40.0));

    let middle = panel.center().x;
    let top = heavy_title(painter, "CONTROLS", middle, panel.min.y + pad, scale) + unit(10.0);

    let mut left = panel.min.x + pad + (inner_width - body.x) / 2.0;
    for (column, size) in columns.iter().zip(&sizes) {
        let mut y = top;
        for (index, (name, rows)) in column.iter().enumerate() {
            if index > 0 {
                y += section_gap;
            }
            // the heading on a slanted plate, black, or a red banner for the attack
            let label = layout(name, 10.0, Color32::WHITE);
            let size = plate(name);
            let points = vec![pos2(left + slant, y), pos2(left + size.x, y), pos2(left + size.x - slant, y + size.y), pos2(left, y + size.y)];
            let (fill, edge) = if *name == "ATTACK" { (RED, Stroke::new(unit(1.4), ORANGE)) } else { (INK, Stroke::NONE) };
            painter.add(Shape::convex_polygon(points, fill, edge));
            painter.galley(pos2(left + plate_pad + slant / 2.0, y + (size.y - label.size().y) / 2.0), label, Color32::WHITE);
            y += plate_h + unit(5.0);
            for row in rows {
                let mut x = left;
                for key in &row.keys {
                    // a key: a white pill on its shadow, red for the aim in use
                    let lit = *name == "AIM" && key == active;
                    let label = layout(key, 11.0, if lit { Color32::WHITE } else { INK });
                    let cap = Rect::from_min_size(pos2(x, y), vec2(label.size().x + 2.0 * cap_pad, cap_h));
                    let round = CornerRadius::same((cap_h / 2.0).round() as u8);
                    painter.rect_filled(cap.translate(vec2(0.0, unit(1.8))), round, PAPER_SHADOW);
                    painter.rect_filled(cap, round, if lit { RED } else { Color32::WHITE });
                    let stroke = if lit { Stroke::new(unit(1.8), ORANGE) } else { Stroke::new(unit(1.1), INK) };
                    painter.rect_stroke(cap, round, stroke, StrokeKind::Inside);
                    painter.galley(cap.center() - label.size() / 2.0, label, INK);
                    x = cap.max.x + gap;
                }
                let does = layout(row.does, 12.0, INK);
                painter.galley(pos2(x + unit(6.0), y + (cap_h - does.size().y) / 2.0), does, INK);
                y += cap_h + row_gap;
            }
        }
        left += size.x + column_gap;
    }

    // the black strip at the foot, as the game explains its menus
    let c = corner.round() as u8;
    let footer = Rect::from_min_max(pos2(panel.min.x, panel.max.y - footer_h), panel.max);
    painter.rect_filled(footer, CornerRadius { nw: 0, ne: 0, sw: c, se: c }, INK);
    painter.galley(footer.center() - tip.size() / 2.0, tip, Color32::WHITE);
    painter.rect_stroke(panel, corner, Stroke::new(unit(1.5), INK), StrokeKind::Inside);
    vec2(width, height)
}

/// the sections split, in order, into this many columns of about as many rows.
fn split_columns<'a>(sections: &'a [(&'a str, Vec<ControlRow>)], count: usize) -> Vec<&'a [(&'a str, Vec<ControlRow>)]> {
    let weight = |(_, rows): &(&str, Vec<ControlRow>)| rows.len() + 1;
    let total: usize = sections.iter().map(weight).sum();
    let mut columns = Vec::new();
    let (mut start, mut taken) = (0, 0);
    for (index, section) in sections.iter().enumerate() {
        taken += weight(section);
        let target = total * (columns.len() + 1) / count;
        if taken >= target && columns.len() + 1 < count && index + 1 < sections.len() {
            columns.push(&sections[start..=index]);
            start = index + 1;
        }
    }
    columns.push(&sections[start..]);
    columns
}

/// a switch for the panel and its setting; false hides both.
pub const CONTROLS_HINT_ENABLED: bool = true;

/// draws the controls panel where it shows biggest, in whichever free
/// rectangle, in points, and however many columns suit it, up to its full
/// size, and nowhere when it would be too small to read. it only paints, so
/// clicks go through to the screens.
pub fn controls_hint(ctx: &egui::Context, free: &[egui::Rect], keys: &Keys, active: &str) {
    const SMALLEST: f32 = 0.6;
    const MARGIN: f32 = 8.0;
    let sections = controls_sections(keys);
    let painter = ctx.layer_painter(egui::LayerId::background());
    let layouts: Vec<_> = (1..=3).map(|count| split_columns(&sections, count)).collect();
    let best = free
        .iter()
        .flat_map(|rect| layouts.iter().map(move |columns| (rect, columns)))
        .map(|(rect, columns)| {
            let full = controls_panel(&painter, columns, 1.0, None, active);
            let scale = ((rect.width() - MARGIN) / full.x).min((rect.height() - MARGIN) / full.y).min(1.0);
            (rect, columns, scale)
        })
        .max_by(|a, b| a.2.total_cmp(&b.2));
    let Some((rect, columns, scale)) = best else { return };
    if scale < SMALLEST {
        return;
    }
    let size = controls_panel(&painter, columns, scale, None, active);
    controls_panel(&painter, columns, scale, Some(rect.center() - size / 2.0), active);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_split_into_columns_in_order_without_losing_any() {
        let sections = controls_sections(&Keys::default());
        for count in 1..=3 {
            let columns = split_columns(&sections, count);
            assert_eq!(columns.len(), count);
            let names: Vec<_> = columns.iter().flat_map(|column| column.iter().map(|(name, _)| *name)).collect();
            assert_eq!(names, ["MOVE", "ATTACK", "POWERS", "AIM", "MENUS"]);
        }
    }
}
