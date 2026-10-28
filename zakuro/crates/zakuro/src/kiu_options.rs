//! Kid Icarus Uprising's options: the settings that make this port nicer to
//! play, gathered in one window drawn as the game's own menus are, ink on
//! parchment under a heavy title. everything else stays in the settings.

use egui::{pos2, vec2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Shape, Stroke, Ui};

use crate::menus::{heavy_title, vine, Action, Menus, INK, PAPER, PAPER_LINE, RED, SHADOW};
use crate::settings::{Filter, Language, Renderer, Screens, Settings};

/// the parchment's margin round the options.
const MARGIN: i8 = 16;
/// narrower than this, the sections go one under another.
const TWO_COLUMNS: f32 = 520.0;
/// the advice at the foot with the space above it, about.
const FOOTER: f32 = 34.0;

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
            .show(ctx, |ui| {
                paper(ui.visuals_mut());
                // the options scroll on a window too short for them, the advice stays at
                // the foot; a window's own scrolling never shrinks to what it holds
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(screen.height() - margin - 2.0 * MARGIN as f32 - FOOTER)
                    .show(ui, |ui| {
                        title(ui, "OPTIONS");
                        if ui.available_width() >= TWO_COLUMNS {
                            ui.columns(2, |columns| {
                                picture(&mut columns[0], settings);
                                mouse(&mut columns[1], settings, &mut actions);
                                game(&mut columns[1], settings);
                            });
                        } else {
                            picture(ui, settings);
                            mouse(ui, settings, &mut actions);
                            game(ui, settings);
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Back").clicked() {
                                self.options_open = false;
                            }
                            if ui.button("All settings").on_hover_text("The rest: controls, the controller, the 3D, the cameras.").clicked() {
                                self.options_open = false;
                                self.settings_open = true;
                            }
                        });
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
            // its longest choice is wider than a column: kept to the column, cut short with "…"
            egui::ComboBox::from_id_salt("kiu screens")
                .width(ui.available_width())
                .truncate()
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

/// the languages Kid Icarus Uprising has: the USA release the first three,
/// the European one all five.
const GAME_LANGUAGES: [Language; 5] = [Language::English, Language::French, Language::Spanish, Language::German, Language::Italian];

/// what the game itself is set to.
fn game(ui: &mut Ui, settings: &mut Settings) {
    ui.add_space(8.0);
    plate(ui, "GAME");
    egui::Grid::new("kiu game").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        row(ui, "Language", "The language the game is in. The USA release has English, French and Spanish, the European one German and Italian too. It changes with Reset or the next start.", |ui| {
            egui::ComboBox::from_id_salt("kiu language")
                .selected_text(settings.language.name())
                .show_ui(ui, |ui| {
                    for language in GAME_LANGUAGES {
                        ui.selectable_value(&mut settings.language, language, language.name());
                    }
                })
                .response
        });
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
        row(ui, "Discord status", "Shows on your Discord profile whether you are in the menus, on foot or flying, the chapter, and for how long. Needs the Discord app open on this computer.", |ui| {
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

/// a window's title as the game's menus draw it, across the window's width.
pub fn title(ui: &mut Ui, text: &str) {
    let (place, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    heavy_title(ui.painter(), text, place.center().x, place.min.y, 1.0);
}

/// the faint ink of hints and second lines, readable on the parchment.
const FADED_INK: Color32 = Color32::from_rgb(98, 86, 74);

/// Kid Icarus' parchment for everything the emulator shows, its windows,
/// menus and notices, as the options window: ink on paper, red when touched,
/// whatever theme the system asks for. one theme set once keeps the
/// emulator's own windows as Zakuro wrote them.
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
        visuals.window_shadow = egui::Shadow { offset: [3, 4], blur: 0, spread: 0, color: SHADOW };
        visuals.popup_shadow = egui::Shadow { offset: [2, 3], blur: 0, spread: 0, color: SHADOW };
        visuals.hyperlink_color = DARK_RED;
        visuals.error_fg_color = DARK_RED;
        visuals.warn_fg_color = Color32::from_rgb(150, 72, 0);
    });
}

/// the banner red darkened to read as text on the parchment.
const DARK_RED: Color32 = Color32::from_rgb(176, 20, 24);

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
        let (ctx, screen) = settled(width, height, Settings::default());
        (ctx.memory(|memory| memory.area_rect(egui::Id::new("kiu options"))).unwrap(), screen)
    }

    /// the context after the options window settled on a screen this size.
    fn settled(width: f32, height: f32, mut settings: Settings) -> (egui::Context, egui::Rect) {
        let ctx = egui::Context::default();
        let mut menus = Menus::default();
        menus.options_open = true;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(width, height));
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                assert!(menus.options(ui.ctx(), &mut settings).is_empty(), "nothing changed, nothing to do");
            })
            .drop_without_applying_deltas();
        }
        (ctx, screen)
    }

    #[test]
    fn the_parchment_ends_at_its_footer() {
        let (ctx, _) = settled(1200.0, 888.0, Settings::default());
        let window = ctx.memory(|memory| memory.area_rect(egui::Id::new("kiu options"))).unwrap();
        let lowest = ctx.viewport(|viewport| {
            viewport.prev_pass.widgets.layers().flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| widget.rect.height() < 40.0)
                .map(|widget| widget.rect.max.y)
                .fold(f32::MIN, f32::max)
        });
        // the footer's advice, its strip's padding and the parchment's shadow below it
        assert!(window.max.y - lowest < 16.0, "{window:?} ends {} below its last line", window.max.y - lowest);
    }

    #[test]
    fn no_option_covers_another() {
        // the longest choice of the screens, at the narrowest two columns and a wide window
        let settings = Settings { layout: Screens::Auto, ..Settings::default() };
        for (width, height) in [(TWO_COLUMNS + 2.0 * MARGIN as f32 + 8.0, 600.0), (1200.0, 888.0)] {
            let (ctx, _) = settled(width, height, settings.clone());
            let widgets: Vec<egui::Rect> = ctx.viewport(|viewport| {
                viewport.prev_pass.widgets.layers().flat_map(|(_, widgets)| widgets.iter())
                    .map(|widget| widget.interact_rect)
                    // the labels and controls as far as they show, not the window and the areas that hold them
                    .filter(|rect| rect.width() < 400.0 && rect.height() < 40.0 && rect.area() > 0.0)
                    .collect()
            });
            for (i, a) in widgets.iter().enumerate() {
                // a widget made of others (a slider and its number) holds them whole
                for b in widgets[i + 1..].iter().filter(|b| !a.contains_rect(**b) && !b.contains_rect(*a)) {
                    assert!(a.intersect(*b).area() < 1.0, "{a:?} covers {b:?} at {width}x{height}");
                }
            }
        }
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

    #[test]
    fn the_pause_menu_is_centred_under_its_title() {
        let ctx = egui::Context::default();
        parchment(&ctx);
        let mut menus = Menus::default();
        menus.menu_open = true;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1200.0, 720.0));
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                menus.game(ui, "Kid Icarus: Uprising", None, true, false, &[], None);
            })
            .drop_without_applying_deltas();
        }
        let buttons: Vec<egui::WidgetRect> = ctx.viewport(|viewport| {
            viewport.prev_pass.widgets.layers().flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| (widget.rect.width() - 200.0).abs() < 0.5)
                .copied()
                .collect()
        });
        assert!(!buttons.is_empty());
        let window = ctx.memory(|memory| memory.area_rect(buttons[0].layer_id.id)).unwrap();
        let buttons = buttons.iter().map(|widget| widget.rect);
        for button in buttons {
            let (left, right) = (button.min.x - window.min.x, window.max.x - button.max.x);
            assert!((left - right).abs() < 4.0, "{button:?} in {window:?}: {left} left, {right} right");
        }
    }

    /// every line of text a shape draws, inside groups too.
    fn texts(shape: &Shape, into: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => into.push(text.galley.text().to_owned()),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, into)),
            _ => {}
        }
    }

    #[test]
    fn the_pause_menu_leaves_the_settings_to_the_options() {
        let ctx = egui::Context::default();
        let mut menus = Menus::default();
        menus.menu_open = true;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1200.0, 720.0));
        // a window's first pass only measures it
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            shapes = ctx.run_ui(input, |ui| {
                menus.game(ui, "Kid Icarus: Uprising", None, true, false, &[], None);
            })
            .shapes;
        }
        let mut shown = Vec::new();
        shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut shown));
        assert!(shown.iter().any(|text| text == "Options"), "{shown:?}");
        assert!(!shown.iter().any(|text| text == "Settings"), "one way in, through the options: {shown:?}");
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
