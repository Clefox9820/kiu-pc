//! the library, the menu over a game and the settings, drawn with egui.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use egui::{Align, Color32, Layout, RichText, Vec2};
use winit::keyboard::KeyCode;
use zakuro_core::services::keyboard::Request;

use crate::library::{Library, ICON_SIZE};
use crate::recompile::{Job, Stage};
use crate::gamepad::button_name;
use crate::settings::{CameraSource, Filter, Keys, Language, PadButtons, Renderer, Screens, Settings, Stereo3d, TopShape};

/// what the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Play(PathBuf),
    /// recompile the library's game at this index.
    Recompile(usize),
    /// download a compiler, then recompile the library's game at this
    /// index with it.
    DownloadCompiler(usize),
    /// show the mods folder of the game with this program id.
    Mods(u64),
    CancelRecompile(u64),
    ChooseFolder,
    ChooseBackground,
    /// pick the picture the cameras see.
    ChooseCameraPicture,
    /// save the game's state to the slot chosen, or load it from there.
    SaveState,
    LoadState,
    /// open a room and join it, join another's, or leave the one joined.
    HostRoom,
    JoinRoom,
    LeaveRoom,
    /// take part in the Download Play offer at this place in the list, or
    /// leave the one taken part in.
    DlpJoin(usize),
    DlpLeave,
    Rescan,
    Resume,
    Reset,
    Library,
    Fullscreen,
    /// copy what a report on the game wants.
    CopyReport,
    Quit,
    /// the settings changed, to save and apply.
    Settings,
    /// close the game's keyboard with this text and the button pressed.
    Keyboard(String, usize),
    /// turn the running game's cheat at this index on or off.
    Cheat(usize, bool),
    /// add a cheat, its name and its code, to the running game.
    AddCheat(String, String),
    /// read the running game's cheats from their file again.
    ReloadCheats,
    /// show the cheats folder.
    CheatsFolder,
}

#[derive(Default)]
pub struct Menus {
    /// the menu over a running game.
    pub menu_open: bool,
    /// the cheats window, and the cheat being written in it.
    pub cheats_open: bool,
    cheat_name: String,
    cheat_code: String,
    pub settings_open: bool,
    /// Kid Icarus' own options, over the menu of the running game.
    pub options_open: bool,
    /// the binding waiting for a key, by its place in Keys::all_mut.
    pub rebinding: Option<usize>,
    /// the controller binding waiting for a button, by its place in
    /// PadButtons::all_mut.
    pub rebinding_pad: Option<usize>,
    /// something to tell the user, until they close it.
    pub message: Option<String>,
    /// something to tell the user for a moment, over the game, and since
    /// when.
    pub toast: Option<(String, std::time::Instant)>,
    /// the slot states are saved to and loaded from, 1 to 9.
    pub slot: u8,
    /// the multiplayer window, and the password typed in it.
    pub multiplayer_open: bool,
    pub room_password: String,
    /// the icons of the games offered for Download Play.
    dlp_icons: HashMap<u64, egui::TextureHandle>,
    /// a report on the game, to copy from the message it goes with, and
    /// from no other.
    pub report: Option<(String, String)>,
    /// what is being typed into the game's keyboard, and what it asked for.
    typing: Option<(Request, String)>,
    /// the library's game to recompile once the player agrees to download a
    /// compiler for it.
    pub compiler_offer: Option<usize>,
    icons: HashMap<PathBuf, egui::TextureHandle>,
    background: Background,
}

/// the room the player is in, as the multiplayer window shows it.
pub struct RoomView {
    pub name: String,
    /// each member's nickname and the game they play.
    pub members: Vec<(String, String)>,
    /// the port of the room this program holds open, when it is its own.
    pub port: Option<u16>,
    /// Download Play, while no game runs.
    pub download_play: Option<DlpView>,
}

/// the Download Play offers in the room, and where the guest is.
pub struct DlpView {
    pub offers: Vec<OfferView>,
    /// what the guest is doing, and how far along, while it takes part.
    pub doing: Option<(String, Option<f32>)>,
}

pub struct OfferView {
    pub child: u64,
    pub title: String,
    pub host: String,
    /// RGBA, ICON_SIZE square.
    pub icon: Option<Vec<u8>>,
    pub guests: u8,
    pub most: u8,
    /// why the player can't take part, when they can't.
    pub blocked: Option<String>,
}

/// the picture behind the library, decoded away from the interface.
#[derive(Default)]
struct Background {
    /// the file shown or being read.
    path: Option<PathBuf>,
    texture: Option<egui::TextureHandle>,
    loading: Option<Receiver<Result<egui::ColorImage, String>>>,
}

/// the longest side a background is kept at, bigger ones are scaled down.
const BACKGROUND_SIDE: u32 = 2560;

impl Background {
    /// the texture for path, starting to read it when it changed, none until
    /// it is ready or when there is no picture.
    fn get(&mut self, ctx: &egui::Context, path: Option<&Path>) -> Result<Option<&egui::TextureHandle>, String> {
        if self.path.as_deref() != path {
            self.path = path.map(Path::to_owned);
            self.texture = None;
            self.loading = path.map(|path| {
                let (send, receive) = channel();
                let path = path.to_owned();
                std::thread::spawn(move || {
                    let _ = send.send(decode(&path));
                });
                receive
            });
        }
        if let Some(receive) = &self.loading {
            match receive.try_recv() {
                Ok(image) => {
                    self.loading = None;
                    self.texture = Some(ctx.load_texture("library background", image?, egui::TextureOptions::LINEAR));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
            }
        }
        Ok(self.texture.as_ref())
    }
}

fn decode(path: &Path) -> Result<egui::ColorImage, String> {
    let image = image::open(path).map_err(|error| format!("could not open {}, {error}", path.display()))?;
    let image = if image.width().max(image.height()) > BACKGROUND_SIDE {
        image.resize(BACKGROUND_SIDE, BACKGROUND_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// the part of a picture that fills an area without stretching it, cutting
/// off what sticks out on either side.
fn cover(area: Vec2, picture: [usize; 2]) -> egui::Rect {
    let (width, height) = (picture[0].max(1) as f32, picture[1].max(1) as f32);
    let area_ratio = area.x / area.y.max(1.0);
    let picture_ratio = width / height;
    if area_ratio > picture_ratio {
        // wider area, the picture's top and bottom are cut
        let shown = picture_ratio / area_ratio;
        egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - shown) / 2.0), egui::pos2(1.0, (1.0 + shown) / 2.0))
    } else {
        let shown = area_ratio / picture_ratio;
        egui::Rect::from_min_max(egui::pos2((1.0 - shown) / 2.0, 0.0), egui::pos2((1.0 + shown) / 2.0, 1.0))
    }
}

impl Menus {
    /// the library, filling the window.
    pub fn library(&mut self, ui: &mut egui::Ui, library: &Library, settings: &Settings, jobs: &[Job]) -> Vec<Action> {
        let mut actions = Vec::new();
        if library.changed {
            self.icons.clear();
        }
        egui::Panel::top("library bar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Zakuro");
                ui.separator();
                match &settings.games {
                    Some(folder) => ui.label(folder.display().to_string()),
                    None => ui.label(RichText::new("no games folder yet").weak()),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        self.settings_open = true;
                    }
                    if ui.button("Multiplayer").clicked() {
                        self.multiplayer_open = true;
                    }
                    if ui.add_enabled(settings.games.is_some(), egui::Button::new("Refresh")).clicked() {
                        actions.push(Action::Rescan);
                    }
                    if ui.button("Choose folder…").clicked() {
                        actions.push(Action::ChooseFolder);
                    }
                });
            });
            ui.add_space(6.0);
        });
        let background = match self.background.get(ui.ctx(), settings.background.as_deref()) {
            Ok(texture) => texture.map(|texture| (texture.id(), texture.size())),
            Err(error) => {
                self.message = Some(error);
                None
            }
        };
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some((texture, size)) = background {
                let area = ui.clip_rect();
                let alpha = (settings.background_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                ui.painter().image(texture, area, cover(area.size(), size), Color32::from_white_alpha(alpha));
            }
            if settings.games.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.label("Pick the folder your games are in to see them here.");
                    if ui.button("Choose folder…").clicked() {
                        actions.push(Action::ChooseFolder);
                    }
                });
                return;
            }
            if library.is_scanning() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Looking for games…");
                });
                return;
            }
            if library.games.is_empty() {
                ui.label("No .3ds, .cci, .cxi or .cia files in that folder.");
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, game) in library.games.iter().enumerate() {
                    let job = jobs.iter().find(|job| job.program_id == game.program_id);
                    // over a picture, each game gets a backing to keep it
                    // readable
                    let mut frame = egui::Frame::group(ui.style());
                    if background.is_some() {
                        frame = frame.fill(ui.visuals().panel_fill.gamma_multiply(0.85));
                    }
                    frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let icon = game.icon.as_ref().map(|pixels| {
                                self.icons.entry(game.path.clone()).or_insert_with(|| {
                                    let image = egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], pixels);
                                    ui.ctx().load_texture(game.path.display().to_string(), image, egui::TextureOptions::LINEAR)
                                })
                            });
                            match icon {
                                Some(texture) => {
                                    ui.add(egui::Image::new(&*texture).fit_to_exact_size(Vec2::splat(ICON_SIZE as f32)));
                                }
                                None => {
                                    ui.allocate_space(Vec2::splat(ICON_SIZE as f32));
                                }
                            }
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&game.name).strong().size(16.0));
                                if !game.publisher.is_empty() {
                                    ui.label(&game.publisher);
                                }
                                if let Some(problem) = &game.problem {
                                    ui.label(RichText::new(problem).color(Color32::from_rgb(230, 160, 80)));
                                    return;
                                }
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("{:016X}", game.program_id)).weak().monospace());
                                    if game.recompiled {
                                        ui.label(RichText::new("recompiled").color(Color32::from_rgb(110, 200, 120)));
                                    }
                                    if game.modded {
                                        ui.label(RichText::new("mods").color(Color32::from_rgb(120, 170, 230)));
                                    }
                                    if let Some(update) = &game.update {
                                        let version = zakuro_fs::version_name(update.version);
                                        ui.label(RichText::new(format!("update {version}")).color(Color32::from_rgb(220, 180, 100)));
                                    }
                                    if !game.dlc.is_empty() {
                                        ui.label(RichText::new("DLC").color(Color32::from_rgb(220, 180, 100)));
                                    }
                                });
                            });
                            if game.problem.is_some() {
                                return;
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button(RichText::new("▶ Play").size(15.0)).clicked() {
                                    actions.push(Action::Play(game.path.clone()));
                                }
                                match job {
                                    Some(job) if !job.stage().finished() => {
                                        if ui.button("Cancel").clicked() {
                                            actions.push(Action::CancelRecompile(job.program_id));
                                        }
                                        progress(ui, job);
                                    }
                                    _ => {
                                        let label = if game.recompiled { "Recompile again" } else { "Recompile" };
                                        if ui.button(label).on_hover_text("Turn the game's code into native code with 3dsrecomp, which makes it run faster. It takes around ten minutes and needs a C compiler, gcc or clang, or Zig, which Zakuro offers to download when the computer has neither.").clicked() {
                                            actions.push(Action::Recompile(index));
                                        }
                                        if let Some(Stage::Failed(error)) = job.map(Job::stage) {
                                            ui.label(RichText::new(format!("failed, {error}")).color(Color32::LIGHT_RED));
                                        }
                                    }
                                }
                                if ui.button("Mods").on_hover_text("Open the game's mods folder. Files put in its romfs folder replace the game's, the way Luma3DS and Citra take mods, and a texture pack made for Citra or Azahar goes in its textures folder.").clicked() {
                                    actions.push(Action::Mods(game.program_id));
                                }
                            });
                        });
                    });
                }
            });
        });
        actions
    }

    /// what goes over a running game, the menu when it is open and the
    /// frame rate when asked for.
    #[allow(clippy::too_many_arguments)]
    pub fn game(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        fps: Option<f32>,
        recompiled: bool,
        fast: bool,
        jobs: &[Job],
        saved: Option<&str>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();
        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2500) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 8.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(text);
                        });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
        if fast {
            egui::Area::new(egui::Id::new("fast forward"))
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-8.0, 8.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new("» fast forward").monospace());
                    });
                });
        }
        if let Some(fps) = fps {
            // clicks go through to the screen under it
            egui::Area::new(egui::Id::new("fps")).fixed_pos(egui::pos2(8.0, 8.0)).interactable(false).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(format!("{fps:.0} fps")).monospace());
                });
            });
        }
        // the options take the menu's place while open
        if !self.menu_open || self.options_open {
            return actions;
        }
        egui::Window::new("Paused")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&ctx, |ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(if recompiled { "running its recompiled code" } else { "interpreted" }).weak());
                ui.add_space(6.0);
                let wide = |ui: &mut egui::Ui, text: &str| ui.add_sized([200.0, 28.0], egui::Button::new(text)).clicked();
                if wide(ui, "Resume") {
                    actions.push(Action::Resume);
                }
                if wide(ui, "Options") {
                    self.options_open = true;
                }
                if wide(ui, "Reset") {
                    actions.push(Action::Reset);
                }
                if wide(ui, "Fullscreen") {
                    actions.push(Action::Fullscreen);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.slot).range(1..=9).prefix("Slot "));
                    ui.label(RichText::new(saved.unwrap_or("empty")).weak());
                });
                ui.horizontal(|ui| {
                    if ui.add_sized([98.0, 28.0], egui::Button::new("Save state")).on_hover_text("F5 saves to this slot while playing.").clicked() {
                        actions.push(Action::SaveState);
                    }
                    if ui
                        .add_enabled(saved.is_some(), egui::Button::new("Load state").min_size(Vec2::new(98.0, 28.0)))
                        .on_hover_text("F8 loads this slot while playing.")
                        .clicked()
                    {
                        actions.push(Action::LoadState);
                    }
                });
                ui.add_space(4.0);
                if wide(ui, "Settings") {
                    self.settings_open = true;
                }
                if wide(ui, "Cheats") {
                    self.cheats_open = true;
                }
                if wide(ui, "Multiplayer") {
                    self.multiplayer_open = true;
                }
                if ui
                    .add_sized([200.0, 28.0], egui::Button::new("Show the camera a picture"))
                    .on_hover_text("Pick a picture for the console's cameras to see, a QR code's say, in place of the computer's camera.")
                    .clicked()
                {
                    actions.push(Action::ChooseCameraPicture);
                }
                if wide(ui, "Copy info for a report") {
                    actions.push(Action::CopyReport);
                }
                if wide(ui, "Back to the library") {
                    actions.push(Action::Library);
                }
                if wide(ui, "Quit") {
                    actions.push(Action::Quit);
                }
                for job in jobs {
                    ui.separator();
                    ui.label(&job.name);
                    progress(ui, job);
                }
            });
        actions
    }

    /// the cheats window over a game, when open, its cheats by name, whether
    /// each is on, and their notes.
    pub fn cheats(&mut self, ctx: &egui::Context, cheats: &[(String, bool, String, bool)]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut open = self.cheats_open;
        egui::Window::new("Cheats").open(&mut open).default_width(380.0).show(ctx, |ui| {
            let mut checkbox = |ui: &mut egui::Ui, index: usize, name: &str, enabled: bool, notes: &str| {
                let mut on = enabled;
                let mut response = ui.checkbox(&mut on, name);
                if !notes.is_empty() {
                    response = response.on_hover_text(notes);
                }
                if response.changed() {
                    actions.push(Action::Cheat(index, on));
                }
            };
            // Zakuro's own, tested on this build of the game
            if cheats.iter().any(|cheat| cheat.3) {
                ui.label(RichText::new("Enhancements").strong());
                for (index, (name, enabled, notes, _)) in cheats.iter().enumerate().filter(|(_, cheat)| cheat.3) {
                    checkbox(ui, index, name, *enabled, "");
                    // what it does to the game and who made it, in sight
                    // before it is turned on
                    ui.indent(index, |ui| ui.label(RichText::new(notes).weak().small()));
                }
                ui.separator();
            }
            if !cheats.iter().any(|cheat| !cheat.3) {
                ui.label(RichText::new("No cheats yet. Add one below, or put the cheat file Citra or Azahar has for this game in the cheats folder.").weak());
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (index, (name, enabled, notes, _)) in cheats.iter().enumerate().filter(|(_, cheat)| !cheat.3) {
                    checkbox(ui, index, name, *enabled, notes);
                }
            });
            ui.separator();
            ui.label("Add a cheat");
            ui.add(egui::TextEdit::singleline(&mut self.cheat_name).hint_text("Name"));
            ui.add(egui::TextEdit::multiline(&mut self.cheat_code).hint_text("00000000 00000000").desired_rows(4).font(egui::TextStyle::Monospace));
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() && !self.cheat_code.trim().is_empty() {
                    actions.push(Action::AddCheat(std::mem::take(&mut self.cheat_name), std::mem::take(&mut self.cheat_code)));
                }
                if ui.button("Reload").on_hover_text("Read the cheats again from their file, after changing it.").clicked() {
                    actions.push(Action::ReloadCheats);
                }
                if ui.button("Open the cheats folder").clicked() {
                    actions.push(Action::CheatsFolder);
                }
            });
        });
        self.cheats_open = open;
        actions
    }

    /// the multiplayer window, when open: the room the player is in, or
    /// joining one and opening one.
    pub fn multiplayer(&mut self, ctx: &egui::Context, settings: &mut Settings, room: Option<&RoomView>) -> Vec<Action> {
        let mut actions = Vec::new();
        let before = settings.clone();
        let mut open = self.multiplayer_open;
        egui::Window::new("Multiplayer").open(&mut open).default_width(380.0).show(ctx, |ui| {
            ui.label(
                RichText::new("Local wireless play with others goes through a room, which one of you opens and the others join over the network.")
                    .weak(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Nickname");
                ui.add(egui::TextEdit::singleline(&mut settings.nickname).char_limit(10).hint_text("Player"))
                    .on_hover_text("The name others see, and the console's own, which games show. It changes with the next game started.");
            });
            ui.separator();
            match room {
                Some(room) => {
                    ui.label(RichText::new(format!("In the room {}", room.name)).strong());
                    if let Some(port) = room.port {
                        ui.label(format!("Open on port {port}. Others join with this computer's address and that port. Over the internet the router has to let that port through."));
                    }
                    ui.add_space(4.0);
                    for (nickname, game) in &room.members {
                        match game.is_empty() {
                            true => ui.label(nickname),
                            false => ui.label(format!("{nickname}, playing {game}")),
                        };
                    }
                    ui.add_space(4.0);
                    let leave = if room.port.is_some() { "Close the room" } else { "Leave the room" };
                    if ui.button(leave).clicked() {
                        actions.push(Action::LeaveRoom);
                    }
                    if let Some(download_play) = &room.download_play {
                        ui.separator();
                        ui.label(RichText::new("Download Play").strong());
                        match &download_play.doing {
                            Some((doing, progress)) => {
                                ui.label(doing);
                                if let Some(progress) = progress {
                                    ui.add(egui::ProgressBar::new(*progress).show_percentage());
                                }
                                if ui.button("Leave").clicked() {
                                    actions.push(Action::DlpLeave);
                                }
                            }
                            None if download_play.offers.is_empty() => {
                                ui.label(RichText::new("No one in the room offers a game to download. A game offers one from its own Download Play menu.").weak());
                            }
                            None => {
                                for (index, offer) in download_play.offers.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        if let Some(pixels) = &offer.icon {
                                            let texture = self.dlp_icons.entry(offer.child).or_insert_with(|| {
                                                let image = egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], pixels);
                                                ui.ctx().load_texture(format!("download play {:016X}", offer.child), image, egui::TextureOptions::LINEAR)
                                            });
                                            ui.add(egui::Image::new(&*texture).fit_to_exact_size(Vec2::splat(32.0)));
                                        }
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(&offer.title).strong());
                                            ui.label(RichText::new(format!("from {}, {} of {} joined", offer.host, offer.guests, offer.most)).weak());
                                        });
                                        if ui.add_enabled(offer.blocked.is_none(), egui::Button::new("Join")).clicked() {
                                            actions.push(Action::DlpJoin(index));
                                        }
                                    });
                                    if let Some(why) = &offer.blocked {
                                        ui.label(RichText::new(why).weak());
                                    }
                                }
                            }
                        }
                    }
                }
                None => {
                    ui.label(RichText::new("Join a room").strong());
                    ui.horizontal(|ui| {
                        ui.label("Address");
                        ui.add(egui::TextEdit::singleline(&mut settings.room_address).hint_text("192.168.0.10:24873"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut self.room_password).password(true).hint_text("none"));
                    });
                    if ui.add_enabled(!settings.room_address.trim().is_empty(), egui::Button::new("Join")).clicked() {
                        actions.push(Action::JoinRoom);
                    }
                    ui.separator();
                    ui.label(RichText::new("Open a room").strong());
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut settings.room_name).hint_text("Zakuro"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        ui.add(egui::DragValue::new(&mut settings.room_port).range(1024..=65535));
                    });
                    ui.label(RichText::new("The password above, when there is one, is the room's.").weak());
                    if ui.button("Open").clicked() {
                        actions.push(Action::HostRoom);
                    }
                }
            }
        });
        self.multiplayer_open = open;
        if *settings != before {
            actions.push(Action::Settings);
        }
        actions
    }

    /// the settings window, when open.
    pub fn settings(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Vec<Action> {
        let mut actions = Vec::new();
        let before = settings.clone();
        let mut open = self.settings_open;
        // taller than a screen has room for, so it scrolls, and it can be
        // dragged to any size that fits
        let room = ctx.content_rect().height() - 16.0;
        egui::Window::new("Settings")
            .id(egui::Id::new("settings"))
            .open(&mut open)
            .vscroll(true)
            .default_size([460.0, room])
            .max_height(room)
            .show(ctx, |ui| {
                ui.heading("Games");
                ui.horizontal(|ui| {
                    match &settings.games {
                        Some(folder) => ui.label(folder.display().to_string()),
                        None => ui.label(RichText::new("none").weak()),
                    };
                    if ui.button("Choose…").clicked() {
                        actions.push(Action::ChooseFolder);
                    }
                });

                ui.separator();
                ui.heading("Graphics");
                egui::ComboBox::from_label("Presented with")
                    .selected_text(match settings.renderer {
                        Renderer::Vulkan => "Vulkan",
                        Renderer::OpenGl => "OpenGL",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.renderer, Renderer::Vulkan, "Vulkan");
                        ui.selectable_value(&mut settings.renderer, Renderer::OpenGl, "OpenGL");
                    });
                ui.checkbox(&mut settings.hardware_rasterizer, "Draw the 3D on the GPU");
                ui.add_enabled(
                    settings.hardware_rasterizer,
                    egui::Slider::new(&mut settings.resolution, 1..=8).text("Resolution").suffix("x"),
                )
                .on_hover_text("How many times the console's resolution the 3D is drawn at. Past what the window shows, the extra pixels smooth the edges, at a cost to the GPU that grows fast.");
                ui.add_enabled(settings.hardware_rasterizer, egui::Checkbox::new(&mut settings.texture_packs, "Texture packs"))
                    .on_hover_text("Draw a game's texture pack, made for Citra or Azahar, put in the textures folder of the game's mods folder.");
                let interpolates = settings.hardware_rasterizer && settings.renderer == Renderer::Vulkan;
                ui.add_enabled(interpolates, egui::Checkbox::new(&mut settings.frame_interpolation, "Frame interpolation"))
                    .on_hover_text("A game at 30 frames a second gets a picture drawn in between each two of its own, with everything halfway between where it was in both, so it moves at 60. The screens show a 60th of a second late. It takes the 3D drawn on the GPU and the Vulkan presenter.");
                ui.add(egui::Slider::new(&mut settings.scale, 1..=6).text("Window scale"));
                egui::ComboBox::from_label("Screens").selected_text(settings.layout.name()).show_ui(ui, |ui| {
                    for layout in Screens::ALL {
                        ui.selectable_value(&mut settings.layout, layout, layout.name());
                    }
                });
                egui::ComboBox::from_label("Filter").selected_text(settings.filter.name()).show_ui(ui, |ui| {
                    for filter in Filter::ALL {
                        ui.selectable_value(&mut settings.filter, filter, filter.name());
                    }
                })
                .response
                .on_hover_text("How the screens' pixels look scaled up: smooth, sharp with soft edges, or plain squares.");
                ui.checkbox(&mut settings.integer_scale, "Scale by whole numbers only")
                    .on_hover_text("Every pixel gets the same size, with a border around the screens.");
                egui::ComboBox::from_label("Top screen shape").selected_text(settings.top_shape.name()).show_ui(ui, |ui| {
                    for shape in TopShape::ALL {
                        ui.selectable_value(&mut settings.top_shape, shape, shape.name());
                    }
                })
                .response
                .on_hover_text("Shows the top screen wider than the console's. Games still draw it at the console's shape, so only turn this on with a widescreen code for the game, which draws a wider scene to match.");
                egui::ComboBox::from_label("3D").selected_text(settings.stereo.name()).show_ui(ui, |ui| {
                    for stereo in Stereo3d::ALL {
                        ui.selectable_value(&mut settings.stereo, stereo, stereo.name());
                    }
                })
                .response
                .on_hover_text("The top screen in 3D, for games that draw it: red and cyan for glasses of those colors, side by side for 3D TVs and headsets, alternate rows for passive 3D monitors. Off, games draw flat, as with the console's 3D slider down.");
                let stereo = settings.stereo != Stereo3d::Off;
                ui.add_enabled(stereo, egui::Slider::new(&mut settings.depth_3d, 0..=100).text("Depth").suffix("%"))
                    .on_hover_text("The console's 3D slider, how deep games draw the 3D.");
                ui.add_enabled(stereo, egui::Checkbox::new(&mut settings.swap_eyes, "Swap eyes"))
                    .on_hover_text("Each eye gets the other's picture, for viewing cross-eyed, glasses worn the other way round, or a monitor whose rows start with the other eye.");
                ui.label(
                    RichText::new("F9 switches them while playing, F10 between the top and the bottom screen alone.").weak(),
                );
                ui.label(RichText::new("The presenter changes the next time Zakuro starts, the 3D with the next game.").weak());

                ui.separator();
                ui.heading("Emulation");
                egui::ComboBox::from_label("Language").selected_text(settings.language.name()).show_ui(ui, |ui| {
                    for language in Language::ALL {
                        ui.selectable_value(&mut settings.language, language, language.name());
                    }
                })
                .response
                .on_hover_text("The console's system language, which titles pick their localised text and files by. It changes with the next game started, or Reset.");
                egui::ComboBox::from_label("Cameras").selected_text(settings.camera.name()).show_ui(ui, |ui| {
                    for source in CameraSource::ALL {
                        ui.selectable_value(&mut settings.camera, source, source.name());
                    }
                })
                .response
                .on_hover_text("What the console's cameras see. The computer's camera turns on only while a game takes pictures.");
                if settings.camera == CameraSource::Picture {
                    ui.horizontal(|ui| {
                        ui.label("Picture");
                        match &settings.camera_picture {
                            Some(path) => ui.label(path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()),
                            None => ui.label(RichText::new("none").weak()),
                        };
                        if ui.button("Choose…").clicked() {
                            actions.push(Action::ChooseCameraPicture);
                        }
                    });
                }
                ui.radio_value(&mut settings.recompiled, true, "Recompiled code when there is some");
                ui.radio_value(&mut settings.recompiled, false, "Interpreter only");
                ui.label(RichText::new("It changes with the next game started, or Reset.").weak());

                ui.separator();
                ui.heading("Sound");
                ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).text("Volume").show_value(false));
                ui.checkbox(&mut settings.mute, "Mute");

                ui.separator();
                ui.heading("Interface");
                ui.checkbox(&mut settings.show_fps, "Show the frame rate");
                if CONTROLS_HINT_ENABLED {
                    ui.checkbox(&mut settings.show_controls, "Show the controls beside the screens");
                }
                ui.horizontal(|ui| {
                    ui.label("Library background");
                    match &settings.background {
                        Some(path) => ui.label(path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()),
                        None => ui.label(RichText::new("none").weak()),
                    };
                    if ui.button("Choose…").clicked() {
                        actions.push(Action::ChooseBackground);
                    }
                    if settings.background.is_some() && ui.button("Remove").clicked() {
                        settings.background = None;
                    }
                });
                ui.add_enabled(
                    settings.background.is_some(),
                    egui::Slider::new(&mut settings.background_opacity, 0.0..=1.0)
                        .text("Opacity")
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                );


                ui.separator();
                ui.heading("Controls");
                egui::Grid::new("controls").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (i, (name, key)) in settings.keys.all_mut().into_iter().enumerate() {
                        ui.label(name);
                        let text = if self.rebinding == Some(i) { "press a key…".to_owned() } else { key_name(*key) };
                        if ui.add_sized([110.0, 20.0], egui::Button::new(text)).clicked() {
                            self.rebinding = Some(i);
                            self.rebinding_pad = None;
                        }
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                if ui.button("Default controls").clicked() {
                    settings.keys = Keys::default();
                    self.rebinding = None;
                }
                ui.label(RichText::new("Drag with the right mouse button to tilt the console, for games that use the motion sensors.").weak());
                ui.add(egui::Slider::new(&mut settings.air_aim_speed, 0.1..=3.0).text("Mouse aim speed, flying (2)"));
                ui.add(egui::Slider::new(&mut settings.land_aim_speed, 0.1..=5.0).text("Mouse aim speed, on foot (3)"));

                ui.separator();
                ui.heading("Controller");
                egui::Grid::new("controller").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (i, (name, button)) in settings.pad.all_mut().into_iter().enumerate() {
                        ui.label(name);
                        let text = if self.rebinding_pad == Some(i) { "press a button…" } else { button_name(*button) };
                        if ui.add_sized([110.0, 20.0], egui::Button::new(text)).clicked() {
                            self.rebinding_pad = Some(i);
                            self.rebinding = None;
                        }
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                ui.label(RichText::new("The circle pad is the left stick, and Home opens the menu over the game.").weak());
                if ui.button("Default controller").clicked() {
                    settings.pad = PadButtons::default();
                    self.rebinding_pad = None;
                }
            });
        self.settings_open = open;
        if !open {
            self.rebinding = None;
            self.rebinding_pad = None;
        }
        if *settings != before {
            actions.push(Action::Settings);
        }
        actions
    }

    /// a message box, when there is something to tell.
    /// the keyboard a game opened to have text typed in, it waits until a
    /// button closes it.
    pub fn keyboard(&mut self, ctx: &egui::Context, request: &Request) -> Option<Action> {
        let fresh = self.typing.as_ref().is_none_or(|(asked, _)| asked != request);
        if fresh {
            self.typing = Some((request.clone(), request.text.clone()));
        }
        let (_, text) = self.typing.as_mut()?;
        let confirm = request.buttons.len() - 1;
        let mut action = None;
        egui::Window::new("Keyboard")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(&request.hint)
                        .char_limit(request.max_length)
                        .desired_width(260.0),
                );
                if fresh {
                    field.request_focus();
                }
                let valid = request.accepts(text);
                ui.horizontal(|ui| {
                    for (button, label) in request.buttons.iter().enumerate() {
                        if ui.add_enabled(button != confirm || valid, egui::Button::new(label)).clicked() {
                            action = Some(Action::Keyboard(text.clone(), button));
                        }
                    }
                });
                // enter confirms, when the text will do
                if valid && field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    action = Some(Action::Keyboard(text.clone(), confirm));
                }
            });
        if action.is_some() {
            self.typing = None;
        }
        action
    }

    /// asks whether to download a compiler, there being none to recompile
    /// with.
    pub fn compiler_offer(&mut self, ctx: &egui::Context) -> Option<Action> {
        let index = self.compiler_offer?;
        let size = crate::zig::download_size().map_or_else(String::new, |size| format!(", {} MB to download and around 400 MB once unpacked", size / 1_000_000));
        let mut action = None;
        egui::Window::new("Recompile")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Recompiling needs a C compiler, and Zakuro didn't find one on this computer. It can download Zig, a compiler that needs no installing, into its own folder{size}."
                ));
                ui.horizontal(|ui| {
                    if ui.button("Download it and recompile").clicked() {
                        action = Some(Action::DownloadCompiler(index));
                        self.compiler_offer = None;
                    }
                    if ui.button("Not now").clicked() {
                        self.compiler_offer = None;
                    }
                });
            });
        action
    }

    pub fn message(&mut self, ctx: &egui::Context) {
        let Some(text) = self.message.clone() else { return };
        egui::Window::new("Zakuro")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(text.as_str());
                ui.horizontal(|ui| {
                    if let Some((_, report)) = self.report.as_ref().filter(|(about, _)| *about == text) {
                        if ui.button("Copy info for a report").clicked() {
                            ui.ctx().copy_text(report.clone());
                        }
                    }
                    if ui.button("OK").clicked() {
                        self.message = None;
                        self.report = None;
                    }
                });
            });
    }
}

/// a recompile's progress bar with where it is.
fn progress(ui: &mut egui::Ui, job: &Job) {
    let stage = job.stage();
    let text = match &stage {
        Stage::Downloading(arrived) => format!("downloading the compiler, {:.0}%", arrived * 100.0),
        Stage::Generating => "finding the code".to_owned(),
        Stage::Compiling { done, total } => format!("compiling {done} of {total}"),
        Stage::Installing => "installing".to_owned(),
        Stage::Done => "done".to_owned(),
        Stage::Failed(error) => format!("failed, {error}"),
    };
    let minutes = job.elapsed().as_secs() / 60;
    let seconds = job.elapsed().as_secs() % 60;
    ui.add(egui::ProgressBar::new(stage.fraction()).desired_width(220.0).text(format!("{text}, {minutes}:{seconds:02}")));
}

/// a key as a person would call it.
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
            row(vec![text("Right click")], "Hold to drag the aim on land"),
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

/// Kid Icarus' menu colours: black ink on parchment, a red banner for what
/// matters most and a magenta to cyan flourish under titles.
pub(crate) const PAPER: Color32 = Color32::from_rgb(243, 235, 221);
pub(crate) const PAPER_LINE: Color32 = Color32::from_rgb(234, 224, 207);
pub(crate) const PAPER_SHADOW: Color32 = Color32::from_rgb(196, 183, 162);
pub(crate) const INK: Color32 = Color32::from_rgb(26, 20, 18);
pub(crate) const RED: Color32 = Color32::from_rgb(222, 32, 36);
pub(crate) const ORANGE: Color32 = Color32::from_rgb(250, 164, 40);
const MAGENTA: Color32 = Color32::from_rgb(232, 64, 196);
const BLUE: Color32 = Color32::from_rgb(84, 110, 240);
const CYAN: Color32 = Color32::from_rgb(64, 216, 232);
pub(crate) const SHADOW: Color32 = Color32::from_black_alpha(110);

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
    painter.add(egui::Shape::convex_polygon(vec![base, middle + side, base + dir * length, middle - side], INK, egui::Stroke::NONE));
}

/// a black vine curling out of a corner, as on the game's parchment menus:
/// it grows from `at` along `along` and curls toward `inward`.
pub(crate) fn vine(painter: &egui::Painter, at: egui::Pos2, along: Vec2, inward: Vec2, size: f32) {
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
        painter.add(egui::Shape::line(points, egui::Stroke::new(width, INK)));
        centre + from * radius
    };
    let stem = |from: egui::Pos2, to: egui::Pos2, bow: Vec2, width: f32| {
        let points = [from, from + (to - from) * 0.4 + bow, to - (to - from) * 0.2 + bow, to];
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, egui::Stroke::new(width, INK)));
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

/// a small heart, the game's coin, centred on `at`.
fn heart(painter: &egui::Painter, at: egui::Pos2, size: f32) {
    use egui::pos2;
    painter.circle_filled(pos2(at.x - size * 0.5, at.y - size * 0.25), size * 0.56, RED);
    painter.circle_filled(pos2(at.x + size * 0.5, at.y - size * 0.25), size * 0.56, RED);
    let points = vec![pos2(at.x - size * 1.04, at.y - size * 0.1), pos2(at.x + size * 1.04, at.y - size * 0.1), pos2(at.x, at.y + size * 1.1)];
    painter.add(egui::Shape::convex_polygon(points, RED, egui::Stroke::NONE));
}

/// a title as the game's menus draw it, centred on `middle` from `top`: heavy
/// ink with a white edge, over a ribbon thick in the middle and thin at the
/// ends, magenta to cyan, curling at both ends round a heart. gives the
/// ribbon's height.
pub(crate) fn heavy_title(painter: &egui::Painter, text: &str, middle: f32, top: f32, scale: f32) -> f32 {
    use egui::{pos2, vec2, FontId, Stroke};
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
    painter.rect_filled(panel.translate(vec2(unit(3.0), unit(4.0))), corner, SHADOW);
    painter.rect_filled(panel, corner, PAPER);
    // the faint ruled lines of the game's parchment
    let mut line = panel.min.y + corner;
    while line < panel.max.y - footer_h {
        painter.line_segment([pos2(panel.min.x + unit(2.0), line), pos2(panel.max.x - unit(2.0), line)], Stroke::new(unit(1.0), PAPER_LINE));
        line += unit(5.0);
    }
    let vine_at = unit(7.0);
    vine(painter, panel.min + vec2(vine_at, vine_at), vec2(1.0, 0.0), vec2(0.0, 1.0), unit(40.0));
    vine(painter, pos2(panel.max.x - vine_at, panel.min.y + vine_at), vec2(-1.0, 0.0), vec2(0.0, 1.0), unit(40.0));

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

/// draws the controls panel where it shows biggest, in whichever free
/// rectangle, in points, and however many columns suit it, up to its full
/// size, and nowhere when it would be too small to read. it only paints, so
/// clicks go through to the screens.
// ponytail: a switch for the panel and its setting; set false to hide both.
pub const CONTROLS_HINT_ENABLED: bool = true;

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
mod controls_tests {
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

pub fn key_name(key: KeyCode) -> String {
    let name = format!("{key:?}");
    for prefix in ["Key", "Digit", "Arrow"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_fills_the_area_without_stretching() {
        // a square picture in a wide area loses its top and bottom
        let wide = cover(Vec2::new(200.0, 100.0), [100, 100]);
        assert_eq!((wide.min.x, wide.max.x), (0.0, 1.0));
        assert_eq!((wide.min.y, wide.max.y), (0.25, 0.75));
        // and in a tall one its sides
        let tall = cover(Vec2::new(100.0, 200.0), [100, 100]);
        assert_eq!((tall.min.x, tall.max.x), (0.25, 0.75));
        assert_eq!((tall.min.y, tall.max.y), (0.0, 1.0));
    }

    #[test]
    fn big_pictures_are_scaled_down_keeping_their_shape() {
        let path = std::env::temp_dir().join(format!("zakuro-background-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(BACKGROUND_SIDE * 2, BACKGROUND_SIDE, image::Rgba([10, 20, 30, 255])).save(&path).unwrap();
        let decoded = decode(&path);
        std::fs::remove_file(&path).ok();
        let decoded = decoded.unwrap();
        assert_eq!(decoded.size, [BACKGROUND_SIDE as usize, BACKGROUND_SIDE as usize / 2]);
        assert_eq!(decoded.pixels[0].to_array(), [10, 20, 30, 255]);
        assert!(decode(Path::new("/nowhere/at/all.png")).is_err());
    }

    /// the settings stay inside a short screen, scrolling in it instead of
    /// running past its bottom.
    #[test]
    fn the_settings_fit_a_short_screen() {
        let ctx = egui::Context::default();
        let mut menus = Menus { settings_open: true, ..Menus::default() };
        let mut settings = Settings::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 360.0));
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                menus.settings(ui.ctx(), &mut settings);
            })
            .drop_without_applying_deltas();
        }
        let window = ctx.memory(|memory| memory.area_rect(egui::Id::new("settings"))).unwrap();
        assert!(screen.contains_rect(window), "{window:?} in {screen:?}");
    }

    #[test]
    fn keys_have_short_names() {
        assert_eq!(key_name(KeyCode::KeyX), "X");
        assert_eq!(key_name(KeyCode::ArrowUp), "Up");
        assert_eq!(key_name(KeyCode::Digit1), "1");
        assert_eq!(key_name(KeyCode::Enter), "Enter");
    }
}
