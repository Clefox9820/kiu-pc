//! mouse aim, after Kid Icarus Uprising's two kinds of battle: the mouse
//! drags the stylus with the pointer hidden and held in the window, or feeds
//! a game override that moves the reticle or turns the camera through the
//! host mailbox; its buttons fire, hold the stylus and use powers. 1, 2 and 3
//! pick it: off, flying, on foot.

use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::KeyCode;
use winit::window::CursorGrabMode;
use zakuro_common::memory_map::{HOST_MAILBOX_LOOK, HOST_MAILBOX_LOOK_ACROSS, HOST_MAILBOX_LOOK_DOWN, HOST_MAILBOX_LOOK_MARK};
use zakuro_core::services::hid::PadState;

use crate::kiu_play::{GAME_FLYING, GAME_ON_FOOT, IN_PLAY_QUIET};

/// how the mouse moves the stylus, after Kid Icarus Uprising's two kinds of
/// battle.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub enum AimMode {
    /// the stylus goes where the mouse does and stops at the edges, the
    /// reticle following it.
    #[default]
    Air,
    /// dragging turns the camera, so an axis that goes too far from the
    /// middle lifts the stylus and puts it back there to drag on.
    Land,
}

impl AimMode {
    /// the modes the digit keys pick, 1 to 3: off, flying, on foot.
    pub const BY_DIGIT: [Option<AimMode>; 3] = [None, Some(AimMode::Air), Some(AimMode::Land)];

    /// the mode a digit key picks, off included; none for any other key.
    pub fn from_digit(key: KeyCode) -> Option<Option<AimMode>> {
        let index = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3].iter().position(|&digit| digit == key)?;
        Some(Self::BY_DIGIT[index])
    }

    /// the digit that picks a mode, off included.
    pub fn digit(mode: Option<AimMode>) -> &'static str {
        match mode {
            Some(AimMode::Air) => "2",
            Some(AimMode::Land) => "3",
            None => "1",
        }
    }

    /// what the player is told on picking a mode, off included.
    pub fn label(mode: Option<AimMode>) -> &'static str {
        match mode {
            Some(AimMode::Air) => "Mouse aim: flying",
            Some(AimMode::Land) => "Mouse aim: on foot",
            None => "Mouse aim off",
        }
    }
}

/// how far land aim drags across from the middle before going back, in
/// bottom screen pixels: nearly the whole screen, as the camera turns by
/// how far a drag goes and each going back costs a few frames of it.
const LAND_REACH: f32 = 140.0;

/// land aim going back to the middle so a drag can go on. lifting a moving
/// stylus is a flick, which spins the camera on, so it holds still first,
/// then lifts, then comes down in the middle.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Wrap {
    Holding(u8),
    Lifted(u8),
}

/// the host mailbox's look mark, HOST_MAILBOX_LOOK_MARK, while mouse look is
/// on foot, "MLK1"; the next two words are the movement not yet taken,
/// across and down, as f32 bottom screen pixels. a game override takes them
/// and writes zeros back.
pub const MOUSE_LOOK: u32 = u32::from_le_bytes(*b"MLK1");
/// the same while flying with a game override that moves the reticle, "MAM1".
pub const MOUSE_AIM: u32 = u32::from_le_bytes(*b"MAM1");

/// the mouse moves the reticle or turns the camera straight, so slow movement
/// is eased for small, precise corrections while a quick sweep still goes
/// far: the gain goes smoothly from AIM_SLOW_GAIN at rest to 1 at AIM_KNEE
/// pixels a frame (after the speed setting).
const AIM_SLOW_GAIN: f32 = 0.5;
const AIM_KNEE: f32 = 8.0;

/// a frame's movement through the flying aim's curve, its direction kept.
fn eased(dx: f32, dy: f32) -> (f32, f32) {
    let t = ((dx * dx + dy * dy).sqrt() / AIM_KNEE).min(1.0);
    let gain = AIM_SLOW_GAIN + (1.0 - AIM_SLOW_GAIN) * t * t * (3.0 - 2.0 * t);
    (dx * gain, dy * gain)
}

/// frames movement may wait in the mailbox untaken before it is dropped, so a
/// menu that never takes it does not spin the camera once play goes on. a
/// game takes it once per tick, every second frame for Kid Icarus Uprising.
const STALE_FRAMES: u8 = 4;

/// frames a wheel notch holds its button, enough for a game to see it.
const WHEEL_FRAMES: u8 = 4;

/// frames the stylus holds still, then stays up, going back.
const HOLD_FRAMES: u8 = 2;
const LIFT_FRAMES: u8 = 2;
/// the bottom screen, in its own pixels.
const TOUCH_WIDTH: f32 = 320.0;
const TOUCH_HEIGHT: f32 = 240.0;

/// the mouse's raw movement as aim, at a speed, its vertical turned round
/// when inverted.
fn aim_delta(delta: (f64, f64), speed: f32, invert_y: bool) -> (f32, f32) {
    let y = if invert_y { -delta.1 } else { delta.1 };
    (delta.0 as f32 * speed, y as f32 * speed)
}

/// what mouse aim holds between frames.
#[derive(Default)]
pub struct MouseAim {
    /// where a stylus that never leaves the bottom screen rests, moved by
    /// the mouse, and how, while it is on.
    at: Option<(f32, f32)>,
    mode: AimMode,
    /// land aim going back to the middle, and the movement that came
    /// meanwhile, put down once it is there.
    wrap: Option<Wrap>,
    pending: (f32, f32),
    /// the right button, which holds land aim's stylus on the screen.
    held: bool,
    /// land aim's movement while the stylus is up, in bottom screen pixels,
    /// for a game override that turns the camera with it, see take_look.
    look: (f32, f32),
    /// frames in a row the mailbox still held movement the game had not taken.
    untaken: u8,
    /// the kind of play the game last said it was in, and frames since.
    game: Option<AimMode>,
    quiet: u32,
    /// the left button, which fires (L) while aiming.
    fire: bool,
    /// buttons the mouse holds while aiming: the middle button, and a wheel
    /// notch, which presses for a few frames.
    mouse_buttons: PadState,
    wheel: Option<(PadState, u8)>,
}

impl MouseAim {
    /// lets go of the buttons, when the game stops seeing the mouse. the
    /// game's mode stays: the same play goes on when the window is back, see
    /// forget_game_mode.
    pub fn release(&mut self) {
        self.fire = false;
        self.held = false;
        self.mouse_buttons = PadState::default();
        self.wheel = None;
    }

    /// forgets the kind of play the game last said, so its next word picks
    /// the aim mode again, as it does once aim was turned off on purpose.
    pub fn forget_game_mode(&mut self) {
        self.game = None;
        self.quiet = 0;
    }

    /// turns mouse aim on in a mode, the stylus down in the middle, or off.
    pub fn set(&mut self, mode: Option<AimMode>) {
        self.at = mode.map(|_| (TOUCH_WIDTH / 2.0, TOUCH_HEIGHT / 2.0));
        self.mode = mode.unwrap_or_default();
        self.wrap = None;
        self.pending = (0.0, 0.0);
        self.fire = false;
        self.held = false;
        self.look = (0.0, 0.0);
    }

    /// the movement saved for mouse look since it was last taken.
    pub fn take_look(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.look)
    }

    /// the aim mode to change to, when what the game says it is doing (a
    /// kiu_play::GAME_ word, 0 for nothing this frame) has changed: on foot,
    /// flying, or off once it has been quiet a while. a mode the player
    /// picked stands until the game changes.
    pub fn game_mode(&mut self, said: u32) -> Option<Option<AimMode>> {
        let mode = match said {
            GAME_ON_FOOT => Some(AimMode::Land),
            GAME_FLYING => Some(AimMode::Air),
            _ => None,
        };
        if let Some(mode) = mode {
            self.quiet = 0;
            return (self.game.replace(mode) != Some(mode)).then_some(Some(mode));
        }
        // nothing to turn off while the game has said nothing
        self.game?;
        // no word from the game for as long as it takes to be out of play
        // is a menu, where aim goes off
        self.quiet += 1;
        (self.quiet > IN_PLAY_QUIET).then(|| {
            self.game = None;
            self.quiet = 0;
            None
        })
    }

    /// fills the host mailbox for mouse look: on foot, the movement since the
    /// last frame added to what the game has not taken; otherwise zeros. the
    /// game's mode word is left alone.
    pub fn feed_mailbox(&mut self, mailbox: &mut [u32]) {
        let mark = match self.mode() {
            Some(AimMode::Land) => MOUSE_LOOK,
            Some(AimMode::Air) if self.flying() => MOUSE_AIM,
            _ => {
                mailbox[HOST_MAILBOX_LOOK].fill(0);
                return;
            }
        };
        let (dx, dy) = self.take_look();
        let (dx, dy) = eased(dx, dy);
        let [mark_word, across_word, down_word] = [HOST_MAILBOX_LOOK_MARK, HOST_MAILBOX_LOOK_ACROSS, HOST_MAILBOX_LOOK_DOWN].map(|index| mailbox[index]);
        let waiting = mark_word == mark && (across_word != 0 || down_word != 0);
        self.untaken = if waiting { self.untaken.saturating_add(1) } else { 0 };
        let keep = waiting && self.untaken <= STALE_FRAMES;
        let carried = |word: u32| if keep { f32::from_bits(word) } else { 0.0 };
        let across = carried(across_word) + dx;
        let down = carried(down_word) + dy;
        mailbox[HOST_MAILBOX_LOOK].copy_from_slice(&[mark, across.to_bits(), down.to_bits()]);
    }

    /// flying, as the game itself said, so an override moves the reticle and
    /// the stylus is not needed.
    fn flying(&self) -> bool {
        self.mode == AimMode::Air && self.game == Some(AimMode::Air)
    }

    /// the stylus is off the screen: on foot when the right button is not
    /// holding it, and flying when an override aims.
    fn up(&self) -> bool {
        (self.mode == AimMode::Land && !self.held) || self.flying()
    }

    pub fn aiming(&self) -> bool {
        self.at.is_some()
    }

    /// drags the stylus by a mouse movement, in bottom screen pixels. up and
    /// down stop at the edges; across, land aim goes back to the middle past
    /// its reach and the air's stops at the edges too.
    pub fn aim_by(&mut self, dx: f32, dy: f32) {
        // land aim drags only while the right button holds the stylus down;
        // otherwise the movement is kept for mouse look
        if self.up() {
            if self.aiming() {
                self.look.0 += dx;
                self.look.1 += dy;
            }
            return;
        }
        if self.wrap.is_some() {
            self.pending.0 += dx;
            self.pending.1 += dy;
            return;
        }
        let wraps = self.mode == AimMode::Land;
        let Some((x, y)) = &mut self.at else { return };
        *y = (*y + dy).clamp(0.0, TOUCH_HEIGHT - 1.0);
        let middle = TOUCH_WIDTH / 2.0;
        let to = *x + dx;
        if wraps && (to - middle).abs() > LAND_REACH {
            *x = middle + LAND_REACH.copysign(to - middle);
            self.wrap = Some(Wrap::Holding(HOLD_FRAMES));
        } else {
            *x = to.clamp(0.0, TOUCH_WIDTH - 1.0);
        }
    }

    /// which way the mouse aims, or none.
    pub fn mode(&self) -> Option<AimMode> {
        self.aiming().then_some(self.mode)
    }

    /// the right button, which puts land aim's stylus down in the middle and
    /// drags it. letting go holds it still first, so lifting is no flick.
    pub fn stylus(&mut self, held: bool) {
        if !self.aiming() || self.mode != AimMode::Land || held == self.held {
            return;
        }
        self.held = held;
        match (held, self.wrap) {
            // pressed again before it lifted: it never left, the drag goes on
            (true, Some(Wrap::Holding(_))) => self.wrap = None,
            // still lifting: it comes down in the middle by itself
            (true, Some(Wrap::Lifted(_))) => {}
            (true, None) => {
                if let Some((x, _)) = &mut self.at {
                    *x = TOUCH_WIDTH / 2.0;
                }
            }
            // already on its way up
            (false, Some(_)) => self.pending = (0.0, 0.0),
            (false, None) => self.wrap = Some(Wrap::Holding(HOLD_FRAMES)),
        }
    }

    pub fn fire(&mut self, pressed: bool) {
        self.fire = pressed && self.aiming();
    }

    /// holds a button with the mouse while aiming, the middle button's.
    pub fn mouse_button(&mut self, button: PadState, pressed: bool) {
        self.mouse_buttons.set(button, pressed && self.aiming());
    }

    /// a wheel notch while aiming, pressing a button for a few frames.
    pub fn wheel(&mut self, button: PadState) {
        if self.aiming() {
            self.wheel = Some((button, WHEEL_FRAMES));
        }
    }

    /// ends a frame the game has seen, putting a lifted stylus down again.
    pub fn frame_done(&mut self) {
        self.wrap = match self.wrap {
            Some(Wrap::Holding(frames)) if frames > 1 => Some(Wrap::Holding(frames - 1)),
            Some(Wrap::Holding(_)) => Some(Wrap::Lifted(LIFT_FRAMES)),
            Some(Wrap::Lifted(frames)) if frames > 1 => Some(Wrap::Lifted(frames - 1)),
            Some(Wrap::Lifted(_)) => {
                // down in the middle, then on with what came meanwhile
                if let Some((x, _)) = &mut self.at {
                    *x = TOUCH_WIDTH / 2.0;
                }
                let (dx, dy) = std::mem::take(&mut self.pending);
                self.wrap = None;
                self.aim_by(dx.clamp(-LAND_REACH, LAND_REACH), dy);
                // which may have gone past its reach again
                self.wrap
            }
            None => None,
        };
        self.wheel = self.wheel.and_then(|(button, frames)| (frames > 1).then_some((button, frames - 1)));
    }

    /// what mouse aim adds to the keyboard's state: the buttons the mouse
    /// holds, and the stylus in place of a click on the bottom screen while
    /// it aims.
    pub fn apply(&self, buttons: &mut PadState, touch: &mut Option<(u16, u16)>) {
        *buttons |= self.mouse_buttons;
        if self.fire {
            buttons.set(PadState::L, true);
        }
        if let Some((button, _)) = self.wheel {
            buttons.set(button, true);
        }
        *touch = match self.at {
            Some(_) if matches!(self.wrap, Some(Wrap::Lifted(_))) => None,
            Some(_) if self.wrap.is_none() && self.up() => None,
            Some((x, y)) => Some((x as u16, y as u16)),
            None => *touch,
        };
    }
}

impl crate::App {
    /// takes the mouse's buttons and wheel while mouse aim has the pointer:
    /// the left button fires, the right holds land aim's stylus, the middle
    /// uses the power, as the D-pad's down does, and the wheel steps through
    /// powers, the D-pad's left and right. says whether it took the event,
    /// which otherwise goes on to the screens.
    pub(crate) fn mouse_aim_event(&mut self, event: &WindowEvent) -> bool {
        let aim = &mut self.keyboard.aim;
        match *event {
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if aim.aiming() => aim.fire(state == ElementState::Pressed),
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } if aim.aiming() => aim.stylus(state == ElementState::Pressed),
            WindowEvent::MouseInput { state, button: MouseButton::Middle, .. } => aim.mouse_button(PadState::DOWN, state == ElementState::Pressed),
            WindowEvent::MouseWheel { delta, .. } => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                    MouseScrollDelta::PixelDelta(position) => position.y > 0.0,
                };
                aim.wheel(if up { PadState::LEFT } else { PadState::RIGHT });
            }
            _ => return false,
        }
        true
    }

    /// the mouse's raw movement, which keeps coming with the pointer locked
    /// in place, moves the aim at the speed the settings give its mode.
    pub(crate) fn mouse_aim_device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = *event {
            let speed = match self.keyboard.aim.mode() {
                Some(AimMode::Air) => self.settings.air_aim_speed,
                _ => self.settings.land_aim_speed,
            };
            let (x, y) = aim_delta(delta, speed, self.settings.invert_aim_y);
            self.keyboard.aim.aim_by(x, y);
        }
    }

    /// 1, 2 and 3 pick the mouse aim mode while a game plays, unless the
    /// controls bind them; says whether the key did.
    pub(crate) fn mouse_aim_key(&mut self, code: KeyCode) -> bool {
        let Some(mode) = AimMode::from_digit(code) else { return false };
        if self.game.is_none() || self.menus.menu_open || self.settings.keys.binds(code) {
            return false;
        }
        self.pick_aim(mode);
        true
    }

    /// changes the mouse aim mode and says so.
    pub(crate) fn pick_aim(&mut self, mode: Option<AimMode>) {
        self.set_mouse_aim(mode);
        self.menus.toast = Some((AimMode::label(mode).to_owned(), std::time::Instant::now()));
    }

    /// mouse aim: the mouse drags the stylus with the pointer hidden and held
    /// in the window, the left button fires.
    pub(crate) fn set_mouse_aim(&mut self, mode: Option<AimMode>) {
        let on = mode.is_some();
        self.keyboard.aim.set(mode);
        if let Some(window) = &self.window {
            let grab = if on { CursorGrabMode::Locked } else { CursorGrabMode::None };
            // Windows has no locked mode, confined does there
            if window.set_cursor_grab(grab).is_err() && on {
                let _ = window.set_cursor_grab(CursorGrabMode::Confined);
            }
            window.set_cursor_visible(!on);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Keyboard;
    use crate::settings::Keys;
    use winit::keyboard::PhysicalKey;

    #[test]
    fn the_digits_pick_the_aim_modes_and_say_which() {
        assert_eq!(AimMode::from_digit(KeyCode::Digit1), Some(None));
        assert_eq!(AimMode::from_digit(KeyCode::Digit2), Some(Some(AimMode::Air)));
        assert_eq!(AimMode::from_digit(KeyCode::Digit3), Some(Some(AimMode::Land)));
        assert_eq!(AimMode::from_digit(KeyCode::Digit4), None);
        for mode in AimMode::BY_DIGIT {
            let digit = AimMode::digit(mode);
            let key = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3][digit.parse::<usize>().unwrap() - 1];
            assert_eq!(AimMode::from_digit(key), Some(mode), "{digit} picks it back");
        }
        assert_eq!(AimMode::label(None), "Mouse aim off");
    }

    #[test]
    fn inverted_aim_turns_only_the_vertical_round() {
        assert_eq!(aim_delta((4.0, 2.0), 1.5, false), (6.0, 3.0));
        assert_eq!(aim_delta((4.0, 2.0), 1.5, true), (6.0, -3.0));
    }

    #[test]
    fn air_aim_follows_the_mouse_and_stops_at_the_edges() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.aim.set(Some(AimMode::Air));
        keyboard.aim.aim_by(10.0, -20.0);
        assert_eq!(keyboard.state().touch, Some((170, 100)));
        keyboard.aim.fire(true);
        assert!(keyboard.state().buttons.contains(PadState::L));
        keyboard.aim.aim_by(500.0, 500.0);
        assert_eq!(keyboard.state().touch, Some((319, 239)), "held at the corner, not back in the middle");
        keyboard.aim.set(None);
        assert_eq!(keyboard.state().touch, None);
        assert!(!keyboard.state().buttons.contains(PadState::L));
    }

    #[test]
    fn land_aim_holds_still_then_lifts_then_goes_on_from_the_middle() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.aim.set(Some(AimMode::Land));
        keyboard.aim.stylus(true);
        keyboard.aim.aim_by(0.0, 20.0);
        keyboard.aim.aim_by(150.0, 0.0);
        // still at the edge of its reach, so lifting is no flick
        assert_eq!(keyboard.state().touch, Some((300, 140)));
        keyboard.aim.aim_by(30.0, 0.0);
        for _ in 0..HOLD_FRAMES {
            assert_eq!(keyboard.state().touch, Some((300, 140)), "holding still, the movement waits");
            keyboard.aim.frame_done();
        }
        for _ in 0..LIFT_FRAMES {
            assert_eq!(keyboard.state().touch, None, "lifted");
            keyboard.aim.frame_done();
        }
        assert_eq!(keyboard.state().touch, Some((190, 140)), "down in the middle, on by what came meanwhile, y kept");
        keyboard.aim.aim_by(0.0, 500.0);
        assert_eq!(keyboard.state().touch, Some((190, 239)), "up and down stop at the edge");
    }

    #[test]
    fn land_aim_drags_only_while_the_right_button_holds_the_stylus() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.aim.set(Some(AimMode::Land));
        keyboard.aim.aim_by(30.0, 0.0);
        assert_eq!(keyboard.state().touch, None, "up until the right button");
        keyboard.aim.stylus(true);
        assert_eq!(keyboard.state().touch, Some((160, 120)), "down in the middle");
        keyboard.aim.aim_by(20.0, 10.0);
        assert_eq!(keyboard.state().touch, Some((180, 130)));
        keyboard.aim.stylus(false);
        keyboard.aim.aim_by(40.0, 0.0);
        for _ in 0..HOLD_FRAMES {
            assert_eq!(keyboard.state().touch, Some((180, 130)), "holds still before lifting, no flick");
            keyboard.aim.frame_done();
        }
        for _ in 0..LIFT_FRAMES + 3 {
            assert_eq!(keyboard.state().touch, None, "stays up");
            keyboard.aim.frame_done();
        }
        keyboard.aim.stylus(true);
        assert_eq!(keyboard.state().touch, Some((160, 130)), "down in the middle again, y kept");
        keyboard.aim.stylus(false);
        keyboard.aim.stylus(true);
        keyboard.aim.aim_by(5.0, 0.0);
        assert_eq!(keyboard.state().touch, Some((165, 130)), "pressed again before lifting, the drag goes on");
        keyboard.key(PhysicalKey::Code(KeyCode::Space), true);
        assert!(keyboard.state().buttons.contains(PadState::L), "Space fires");
    }

    #[test]
    fn land_aim_without_the_stylus_saves_the_movement_for_mouse_look() {
        let mut aim = MouseAim::default();
        aim.set(Some(AimMode::Land));
        aim.aim_by(10.0, -4.0);
        aim.aim_by(5.0, 1.0);
        let (mut buttons, mut touch) = (PadState::default(), None);
        aim.apply(&mut buttons, &mut touch);
        assert_eq!(touch, None, "the stylus stays up");
        assert_eq!(aim.take_look(), (15.0, -3.0));
        assert_eq!(aim.take_look(), (0.0, 0.0), "taken once");
        aim.stylus(true);
        aim.aim_by(10.0, 0.0);
        assert_eq!(aim.take_look(), (0.0, 0.0), "a held stylus drags instead");
        aim.set(Some(AimMode::Air));
        aim.aim_by(10.0, 0.0);
        assert_eq!(aim.take_look(), (0.0, 0.0), "only on foot");
    }

    #[test]
    fn mouse_look_waits_in_the_mailbox_until_the_game_takes_it() {
        let mut aim = MouseAim::default();
        let mut mailbox = [7u32; 4];
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox, [0, 0, 0, 7], "aim off writes nothing, the game's word is left");
        aim.set(Some(AimMode::Land));
        aim.aim_by(10.0, -2.0);
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox[..3], [MOUSE_LOOK, 10f32.to_bits(), (-2f32).to_bits()]);
        // quick movement, past the easing curve's knee, so the numbers stay
        aim.aim_by(AIM_KNEE, 0.0);
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), 10.0 + AIM_KNEE, "not taken yet, so it adds up");
        mailbox[1] = 0;
        mailbox[2] = 0;
        aim.aim_by(500.0, 0.0);
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), 500.0, "fast movement is not cut short");
        for _ in 0..STALE_FRAMES {
            aim.aim_by(AIM_KNEE, 0.0);
            aim.feed_mailbox(&mut mailbox);
        }
        assert_eq!(f32::from_bits(mailbox[1]), 500.0 + AIM_KNEE * STALE_FRAMES as f32, "a game tick can take a few frames");
        aim.aim_by(AIM_KNEE, 0.0);
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), AIM_KNEE, "left untaken longer, as in a menu, it is dropped");
        aim.set(Some(AimMode::Air));
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox, [0, 0, 0, 7], "only on foot");
    }

    #[test]
    fn the_game_mode_override_picks_the_aim_mode() {
        let mut aim = MouseAim::default();
        assert_eq!(aim.game_mode(0), None, "no override, nothing changes");
        assert_eq!(aim.game_mode(GAME_ON_FOOT), Some(Some(AimMode::Land)));
        assert_eq!(aim.game_mode(0), None, "between ticks");
        assert_eq!(aim.game_mode(GAME_ON_FOOT), None, "already on foot, a key the player pressed stands");
        assert_eq!(aim.game_mode(GAME_FLYING), Some(Some(AimMode::Air)));
        for _ in 0..IN_PLAY_QUIET {
            assert_eq!(aim.game_mode(0), None);
        }
        assert_eq!(aim.game_mode(0), Some(None), "quiet for long, a menu: aim off");
        assert_eq!(aim.game_mode(0), None, "said once");
        assert_eq!(aim.game_mode(GAME_FLYING), Some(Some(AimMode::Air)), "back in play");
    }

    /// the keyboard let go when the window loses the focus is no change of
    /// play: coming back to the same battle leaves the aim as the player and
    /// the settings had it, but turning aim off on purpose has the game's
    /// word pick it again.
    #[test]
    fn letting_go_of_the_keys_keeps_the_game_mode() {
        let mut keyboard = Keyboard::new(Keys::default());
        assert_eq!(keyboard.aim.game_mode(GAME_FLYING), Some(Some(AimMode::Air)));
        keyboard.release();
        assert_eq!(keyboard.aim.game_mode(GAME_FLYING), None, "the focus back in the same play");
        keyboard.aim.forget_game_mode();
        assert_eq!(keyboard.aim.game_mode(GAME_FLYING), Some(Some(AimMode::Air)), "told again after the menu");
    }

    #[test]
    fn flying_aims_through_the_mailbox_once_the_game_says_it_flies() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.aim.set(Some(AimMode::Air));
        keyboard.aim.aim_by(10.0, 0.0);
        assert_eq!(keyboard.state().touch, Some((170, 120)), "without the game's word, the stylus as before");
        assert_eq!(keyboard.aim.game_mode(GAME_FLYING), Some(Some(AimMode::Air)));
        keyboard.aim.set(Some(AimMode::Air));
        keyboard.aim.aim_by(10.0, -4.0);
        assert_eq!(keyboard.state().touch, None, "the stylus stays up");
        let mut mailbox = [0u32; 4];
        keyboard.aim.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox[..3], [MOUSE_AIM, 10f32.to_bits(), (-4f32).to_bits()]);
    }

    #[test]
    fn flying_aim_eases_slow_movement_and_keeps_quick_sweeps() {
        let (x, y) = eased(1.0, 0.0);
        assert!(x >= AIM_SLOW_GAIN && x < AIM_SLOW_GAIN + 0.05 && y == 0.0, "slow: about the slow gain, {x}");
        assert_eq!(eased(AIM_KNEE * 2.0, 0.0), (AIM_KNEE * 2.0, 0.0), "quick: untouched");
        let (x, y) = eased(3.0, 4.0);
        assert!((y / x - 4.0 / 3.0).abs() < 1e-5, "the direction is kept");
        let mut previous = 0.0;
        for step in 1..=40 {
            let moved = eased(step as f32 * 0.5, 0.0).0;
            assert!(moved > previous, "more movement always moves further");
            previous = moved;
        }
        let mut aim = MouseAim::default();
        aim.game_mode(GAME_FLYING);
        aim.set(Some(AimMode::Air));
        aim.aim_by(1.0, 0.0);
        let mut mailbox = [0u32; 4];
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), eased(1.0, 0.0).0, "flying feeds eased movement");
        aim.set(Some(AimMode::Land));
        aim.aim_by(1.0, 0.0);
        aim.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), eased(1.0, 0.0).0, "on foot the camera eases the same way");
    }

    #[test]
    fn a_wheel_notch_presses_for_a_few_frames_while_aiming() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.aim.wheel(PadState::RIGHT);
        assert!(!keyboard.state().buttons.contains(PadState::RIGHT), "not without aim");
        keyboard.aim.set(Some(AimMode::Land));
        keyboard.aim.wheel(PadState::RIGHT);
        for _ in 0..WHEEL_FRAMES {
            assert!(keyboard.state().buttons.contains(PadState::RIGHT));
            keyboard.aim.frame_done();
        }
        assert!(!keyboard.state().buttons.contains(PadState::RIGHT));
        keyboard.aim.mouse_button(PadState::DOWN, true);
        assert!(keyboard.state().buttons.contains(PadState::DOWN));
    }
}
