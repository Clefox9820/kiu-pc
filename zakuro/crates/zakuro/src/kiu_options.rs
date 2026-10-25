//! Kid Icarus Uprising's options: the settings that make this port nicer to
//! play, gathered in one window drawn as the game's own menus are, ink on
//! parchment under a heavy title. everything else stays in the settings.

use egui::{pos2, vec2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Shape, Stroke, Ui};

use crate::menus::{heavy_title, vine, Action, Menus, INK, PAPER, PAPER_LINE, RED, SHADOW};
use crate::settings::{Filter, Renderer, Screens, Settings};

/// the parchment's margin round the options.
const MARGIN: i8 = 16;
/// narrower than this, the sections go one under another.
const TWO_COLUMNS: f32 = 520.0;

impl Menus {
    /// the options window, when open, over a running game's menu.
    pub fn options(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.options_open {
            return actions;
        }
        let before = settings.clone();
        let frame = egui::Frame::new()
            .fill(PAPER)
            .stroke(Stroke::new(1.5, INK))
            .corner_radius(8)
            .inner_margin(Margin::same(MARGIN))
            .shadow(egui::Shadow { offset: [3, 4], blur: 0, spread: 0, color: SHADOW });
        let screen = ctx.content_rect();
        let margin = 2.0 * MARGIN as f32 + 8.0;
        let width = (screen.width() - margin).clamp(240.0, 560.0);
        egui::Window::new("Kid Icarus options")
            .id(egui::Id::new("kiu options"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(frame)
            .min_width(width)
            .max_width(width)
            .vscroll(true)
            .max_height(screen.height() - margin)
            .show(ctx, |ui| {
                paper(ui.visuals_mut());
                let (title, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                heavy_title(ui.painter(), "OPTIONS", title.center().x, title.min.y, 1.0);
                if ui.available_width() >= TWO_COLUMNS {
                    ui.columns(2, |columns| {
                        picture(&mut columns[0], settings);
                        mouse(&mut columns[1], settings, &mut actions);
                    });
                } else {
                    picture(ui, settings);
                    mouse(ui, settings, &mut actions);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.options_open = false;
                    }
                    if ui.button("All settings").on_hover_text("The rest: controls, the controller, the 3D, the language.").clicked() {
                        self.options_open = false;
                        self.settings_open = true;
                    }
                });
                ui.add_space(8.0);
                footer(ui, "Esc goes back. Each change is kept as it is made.");
                // the vines in the parchment's top corners
                let edge = ui.min_rect().expand(MARGIN as f32);
                let painter = ui.painter().with_clip_rect(ui.ctx().content_rect());
                vine(&painter, edge.min + vec2(7.0, 7.0), vec2(1.0, 0.0), vec2(0.0, 1.0), 40.0);
                vine(&painter, pos2(edge.max.x - 7.0, edge.min.y + 7.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), 40.0);
            });
        if *settings != before {
            actions.push(Action::Settings);
        }
        actions
    }
}

/// what shows and how it sounds.
fn picture(ui: &mut Ui, settings: &mut Settings) {
    plate(ui, "PICTURE");
    egui::Grid::new("kiu picture").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        let gpu = settings.hardware_rasterizer;
        row(ui, "Resolution", "How many times the console's resolution the 3D is drawn at. It changes with Reset or the next start.", |ui| {
            ui.add_enabled(gpu, egui::Slider::new(&mut settings.resolution, 1..=8).suffix("x"))
        });
        let interpolates = gpu && settings.renderer == Renderer::Vulkan;
        row(ui, "Smoother motion", "A picture drawn between each two of the game's 30 a second, so it moves at 60. It shows a 60th of a second late.", |ui| {
            ui.add_enabled(interpolates, egui::Checkbox::without_text(&mut settings.frame_interpolation))
        });
        row(ui, "Screens", "Where the two screens go. F9 switches them while playing.", |ui| {
            egui::ComboBox::from_id_salt("kiu screens")
                .selected_text(settings.layout.name())
                .show_ui(ui, |ui| {
                    for layout in Screens::ALL {
                        ui.selectable_value(&mut settings.layout, layout, layout.name());
                    }
                })
                .response
        });
        row(ui, "Filter", "How the screens' pixels look scaled up.", |ui| {
            egui::ComboBox::from_id_salt("kiu filter")
                .selected_text(settings.filter.name())
                .show_ui(ui, |ui| {
                    for filter in Filter::ALL {
                        ui.selectable_value(&mut settings.filter, filter, filter.name());
                    }
                })
                .response
        });
        row(ui, "Whole pixels", "Every pixel the same size, with a border round the screens.", |ui| {
            ui.checkbox(&mut settings.integer_scale, "")
        });
        row(ui, "Frame rate", "Shows the frames a second in a corner.", |ui| ui.checkbox(&mut settings.show_fps, ""));
    });
    ui.add_space(8.0);
    plate(ui, "SOUND");
    egui::Grid::new("kiu sound").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        row(ui, "Volume", "How loud the game is.", |ui| {
            ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).custom_formatter(|value, _| format!("{:.0}%", value * 100.0)))
        });
        row(ui, "Mute", "No sound at all.", |ui| ui.checkbox(&mut settings.mute, ""));
    });
}

/// how the mouse aims and what the window does in the background.
fn mouse(ui: &mut Ui, settings: &mut Settings, actions: &mut Vec<Action>) {
    plate(ui, "MOUSE");
    egui::Grid::new("kiu mouse").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        row(ui, "Speed, flying", "How far the aim moves for the mouse's movement, with mouse aim 2.", |ui| {
            ui.add(egui::Slider::new(&mut settings.air_aim_speed, 0.1..=3.0))
        });
        row(ui, "Speed, on foot", "How fast the camera turns for the mouse's movement, with mouse aim 3.", |ui| {
            ui.add(egui::Slider::new(&mut settings.land_aim_speed, 0.1..=5.0))
        });
        row(ui, "Invert up and down", "The aim goes up when the mouse goes down, as in flight games.", |ui| {
            ui.checkbox(&mut settings.invert_aim_y, "")
        });
    });
    ui.add_space(8.0);
    plate(ui, "WINDOW");
    egui::Grid::new("kiu window").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        row(ui, "Pause in the background", "The game stops while another window is in front, and goes on when this one is back. Never in a multiplayer room.", |ui| {
            ui.checkbox(&mut settings.pause_unfocused, "")
        });
        row(ui, "Take the mouse back", "Coming back to the window brings back the mouse aim it had, 2 or 3, without pressing it again.", |ui| {
            ui.checkbox(&mut settings.recapture_mouse, "")
        });
        row(ui, "Discord status", "Shows on your Discord profile whether you are in the menus, on foot or flying, and for how long. Needs the Discord app open on this computer.", |ui| {
            ui.checkbox(&mut settings.discord_presence, "")
        });
        row(ui, "Fullscreen", "F11 does it too.", |ui| {
            let response = ui.button("Switch");
            if response.clicked() {
                actions.push(Action::Fullscreen);
            }
            response
        });
    });
}

/// a line of a section: its name, then its control, both explained on hover.
fn row(ui: &mut Ui, name: &str, hint: &str, add: impl FnOnce(&mut Ui) -> egui::Response) {
    ui.label(name).on_hover_text(hint);
    add(ui).on_hover_text(hint);
    ui.end_row();
}

/// a section's heading, white on a slanted black plate.
fn plate(ui: &mut Ui, name: &str) {
    let (slant, pad) = (4.0, 7.0);
    let galley = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(11.0), Color32::WHITE);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(2.0 * pad + slant, 4.0), Sense::hover());
    let points = vec![
        pos2(rect.min.x + slant, rect.min.y),
        rect.right_top(),
        pos2(rect.max.x - slant, rect.max.y),
        rect.left_bottom(),
    ];
    ui.painter().add(Shape::convex_polygon(points, INK, Stroke::NONE));
    ui.painter().galley(pos2(rect.min.x + pad + slant / 2.0, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    ui.add_space(4.0);
}

/// the black strip of advice at the foot, out to the parchment's edges.
fn footer(ui: &mut Ui, text: &str) {
    let corner = CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 };
    egui::Frame::new()
        .fill(INK)
        .corner_radius(corner)
        .inner_margin(Margin::symmetric(MARGIN, 5))
        .outer_margin(Margin { left: -MARGIN, right: -MARGIN, top: 0, bottom: -MARGIN })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(Color32::WHITE).small());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// the options window on a screen this size, after it settles.
    fn shown(width: f32, height: f32) -> (egui::Rect, egui::Rect) {
        let ctx = egui::Context::default();
        let mut menus = Menus::default();
        menus.options_open = true;
        let mut settings = Settings::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(width, height));
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                assert!(menus.options(ui.ctx(), &mut settings).is_empty(), "nothing changed, nothing to do");
            })
            .drop_without_applying_deltas();
        }
        (ctx.memory(|memory| memory.area_rect(egui::Id::new("kiu options"))).unwrap(), screen)
    }

    #[test]
    fn the_options_fit_the_smallest_window_and_a_wide_one() {
        // the console's screens stacked at scale 1, and a 720p window
        for (width, height) in [(400.0, 480.0), (1280.0, 720.0)] {
            let (window, screen) = shown(width, height);
            assert!(screen.contains_rect(window), "{window:?} in {screen:?}");
        }
    }

    #[test]
    fn the_options_open_tall_enough_to_read_on_a_big_window() {
        let (window, _) = shown(1920.0, 1080.0);
        assert!(window.height() > 300.0, "{window:?}");
    }

    #[test]
    fn closed_options_show_nothing() {
        let ctx = egui::Context::default();
        let mut menus = Menus::default();
        let mut settings = Settings::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            menus.options(ui.ctx(), &mut settings);
        })
        .drop_without_applying_deltas();
        assert!(ctx.memory(|memory| memory.area_rect(egui::Id::new("kiu options"))).is_none());
    }
}
