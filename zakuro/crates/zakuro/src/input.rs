//! keyboard to 3DS button mapping.

use winit::keyboard::{KeyCode, PhysicalKey};
use zakuro_core::services::hid::{InputState, PadState};

use crate::settings::Keys;

/// accumulates key state between frames.
#[derive(Default)]
pub struct Keyboard {
    keys: Keys,
    buttons: PadState,
    /// the circle pad keys held, up, down, left, right.
    circle: [bool; 4],
    /// the C-stick's, the same way.
    c_stick: [bool; 4],
    touch: Option<(u16, u16)>,
    /// how the mouse tilts the console, see InputState::tilt.
    tilt: [f32; 2],
    /// mouse aim: where a stylus that never leaves the bottom screen rests,
    /// moved by the mouse, and how, while it is on.
    aim: Option<(f32, f32)>,
    aim_mode: AimMode,
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
    quiet: u8,
    /// the left button, which fires (L) while aiming.
    fire: bool,
    /// Shift, which pushes the circle pad halfway, to walk.
    walk: bool,
    /// buttons the mouse holds while aiming: the middle button, and a wheel
    /// notch, which presses for a few frames.
    mouse_buttons: PadState,
    wheel: Option<(PadState, u8)>,
}

/// frames a wheel notch holds its button, enough for a game to see it.
const WHEEL_FRAMES: u8 = 4;

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

/// the host mailbox's first word while mouse look is on foot, "MLK1"; the
/// next two are the movement not yet taken, across and down, as f32 bottom
/// screen pixels. a game override takes them and writes zeros back.
pub const MOUSE_LOOK: u32 = u32::from_le_bytes(*b"MLK1");
/// the same while flying with a game override that moves the reticle, "MAM1".
pub const MOUSE_AIM: u32 = u32::from_le_bytes(*b"MAM1");

/// the host mailbox's fourth word, which game overrides set every tick to the
/// kind of play going on, and the frontend reads and clears every frame.
pub const GAME_ON_FOOT: u32 = 1;
pub const GAME_FLYING: u32 = 2;

/// frames with no word from the game before aim goes off, as in a menu. the
/// overrides say it once per tick, every second frame for Kid Icarus.
const QUIET_FRAMES: u8 = 10;

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

/// frames the stylus holds still, then stays up, going back.
const HOLD_FRAMES: u8 = 2;
const LIFT_FRAMES: u8 = 2;
/// the bottom screen, in its own pixels.
const TOUCH_WIDTH: f32 = 320.0;
const TOUCH_HEIGHT: f32 = 240.0;

impl Keyboard {
    pub fn new(keys: Keys) -> Keyboard {
        Keyboard { keys, ..Keyboard::default() }
    }

    pub fn set_keys(&mut self, keys: Keys) {
        self.keys = keys;
        self.release();
    }

    /// lets go of everything, when the game stops seeing the keyboard.
    pub fn release(&mut self) {
        self.buttons = PadState::default();
        self.circle = [false; 4];
        self.c_stick = [false; 4];
        self.touch = None;
        self.tilt = [0.0; 2];
        self.fire = false;
        self.held = false;
        self.walk = false;
        // the game's mode is told again once the keyboard is back
        self.game = None;
        self.quiet = 0;
        self.mouse_buttons = PadState::default();
        self.wheel = None;
    }

    /// turns mouse aim on in a mode, the stylus down in the middle, or off.
    pub fn set_aim(&mut self, mode: Option<AimMode>) {
        self.aim = mode.map(|_| (TOUCH_WIDTH / 2.0, TOUCH_HEIGHT / 2.0));
        self.aim_mode = mode.unwrap_or_default();
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
    /// GAME_ word, 0 for nothing this frame) has changed: on foot, flying,
    /// or off once it has been quiet a while. a mode the player picked stands
    /// until the game changes.
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
        if self.game.is_none() {
            return None;
        }
        self.quiet += 1;
        (self.quiet > QUIET_FRAMES).then(|| {
            self.game = None;
            self.quiet = 0;
            None
        })
    }

    /// fills the host mailbox for mouse look: on foot, the movement since the
    /// last frame added to what the game has not taken; otherwise zeros. the
    /// game's mode word is left alone.
    pub fn feed_mailbox(&mut self, mailbox: &mut [u32]) {
        let mark = match self.aim_mode() {
            Some(AimMode::Land) => MOUSE_LOOK,
            Some(AimMode::Air) if self.flying() => MOUSE_AIM,
            _ => {
                mailbox[..3].fill(0);
                return;
            }
        };
        let (dx, dy) = self.take_look();
        let (dx, dy) = eased(dx, dy);
        let waiting = mailbox[0] == mark && (mailbox[1] != 0 || mailbox[2] != 0);
        self.untaken = if waiting { self.untaken.saturating_add(1) } else { 0 };
        let keep = waiting && self.untaken <= STALE_FRAMES;
        let carried = |word: u32| if keep { f32::from_bits(word) } else { 0.0 };
        let across = carried(mailbox[1]) + dx;
        let down = carried(mailbox[2]) + dy;
        mailbox[..3].copy_from_slice(&[mark, across.to_bits(), down.to_bits()]);
    }

    /// flying, as the game itself said, so an override moves the reticle and
    /// the stylus is not needed.
    fn flying(&self) -> bool {
        self.aim_mode == AimMode::Air && self.game == Some(AimMode::Air)
    }

    /// the stylus is off the screen: on foot when the right button is not
    /// holding it, and flying when an override aims.
    fn up(&self) -> bool {
        (self.aim_mode == AimMode::Land && !self.held) || self.flying()
    }

    pub fn aiming(&self) -> bool {
        self.aim.is_some()
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
        let wraps = self.aim_mode == AimMode::Land;
        let Some((x, y)) = &mut self.aim else { return };
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
    pub fn aim_mode(&self) -> Option<AimMode> {
        self.aiming().then_some(self.aim_mode)
    }

    /// the right button, which puts land aim's stylus down in the middle and
    /// drags it. letting go holds it still first, so lifting is no flick.
    pub fn stylus(&mut self, held: bool) {
        if !self.aiming() || self.aim_mode != AimMode::Land || held == self.held {
            return;
        }
        self.held = held;
        match (held, self.wrap) {
            // pressed again before it lifted: it never left, the drag goes on
            (true, Some(Wrap::Holding(_))) => self.wrap = None,
            // still lifting: it comes down in the middle by itself
            (true, Some(Wrap::Lifted(_))) => {}
            (true, None) => {
                if let Some((x, _)) = &mut self.aim {
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
                if let Some((x, _)) = &mut self.aim {
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

    fn button_for(&self, key: KeyCode) -> Option<PadState> {
        let keys = &self.keys;
        [
            (keys.a, PadState::A),
            (keys.b, PadState::B),
            (keys.x, PadState::X),
            (keys.y, PadState::Y),
            (keys.l, PadState::L),
            (keys.r, PadState::R),
            (keys.start, PadState::START),
            (keys.select, PadState::SELECT),
            (keys.up, PadState::UP),
            (keys.down, PadState::DOWN),
            (keys.left, PadState::LEFT),
            (keys.right, PadState::RIGHT),
            (keys.zl, PadState::ZL),
            (keys.zr, PadState::ZR),
        ]
        .into_iter()
        .find(|&(bound, _)| bound == key)
        .map(|(_, button)| button)
    }

    pub fn key(&mut self, key: PhysicalKey, pressed: bool) {
        let PhysicalKey::Code(code) = key else {
            return;
        };
        if let Some(button) = self.button_for(code) {
            self.buttons.set(button, pressed);
        } else if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
            self.walk = pressed;
        }
        let circle = [self.keys.circle_up, self.keys.circle_down, self.keys.circle_left, self.keys.circle_right];
        let c_stick = [self.keys.c_up, self.keys.c_down, self.keys.c_left, self.keys.c_right];
        for (held, bound) in self.circle.iter_mut().zip(circle).chain(self.c_stick.iter_mut().zip(c_stick)) {
            if bound == code {
                *held = pressed;
            }
        }
    }

    /// records a click on the bottom screen, in screen pixels, or its release.
    pub fn touch(&mut self, position: Option<(u16, u16)>) {
        self.touch = position;
    }

    /// records the tilt the mouse gives the console.
    pub fn tilt(&mut self, tilt: [f32; 2]) {
        self.tilt = tilt;
    }

    pub fn state(&self) -> InputState {
        // holding two opposite keys cancels out, which is what a real stick
        // would do
        let axis = |plus: bool, minus: bool| plus as i8 as f32 - minus as i8 as f32;
        let pace = if self.walk { 0.5 } else { 1.0 };
        let [up, down, left, right] = self.circle;
        let [c_up, c_down, c_left, c_right] = self.c_stick;
        let mut buttons = self.buttons | self.mouse_buttons;
        if self.fire {
            buttons.set(PadState::L, true);
        }
        if let Some((button, _)) = self.wheel {
            buttons.set(button, true);
        }
        let touch = match self.aim {
            Some(_) if matches!(self.wrap, Some(Wrap::Lifted(_))) => None,
            Some(_) if self.wrap.is_none() && self.up() => None,
            Some((x, y)) => Some((x as u16, y as u16)),
            None => self.touch,
        };
        InputState {
            buttons,
            circle_x: axis(right, left) * pace,
            circle_y: axis(up, down) * pace,
            c_stick_x: axis(c_right, c_left),
            c_stick_y: axis(c_up, c_down),
            touch,
            tilt: self.tilt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_press_the_buttons_they_are_bound_to() {
        let mut keyboard = Keyboard::new(Keys { a: KeyCode::KeyK, ..Keys::default() });
        keyboard.key(PhysicalKey::Code(KeyCode::KeyK), true);
        assert!(keyboard.state().buttons.contains(PadState::A));
        keyboard.key(PhysicalKey::Code(KeyCode::Enter), true);
        assert!(!keyboard.state().buttons.contains(PadState::B), "Enter is no longer bound to anything");
    }

    #[test]
    fn the_c_stick_keys_push_the_c_stick() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.key(PhysicalKey::Code(KeyCode::Numpad8), true);
        keyboard.key(PhysicalKey::Code(KeyCode::Numpad4), true);
        keyboard.key(PhysicalKey::Code(KeyCode::KeyB), true);
        let state = keyboard.state();
        assert_eq!((state.c_stick_x, state.c_stick_y), (-1.0, 1.0));
        assert_eq!((state.circle_x, state.circle_y), (0.0, 0.0));
        assert!(state.buttons.contains(PadState::ZR));
    }

    #[test]
    fn air_aim_follows_the_mouse_and_stops_at_the_edges() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.set_aim(Some(AimMode::Air));
        keyboard.aim_by(10.0, -20.0);
        assert_eq!(keyboard.state().touch, Some((170, 100)));
        keyboard.fire(true);
        assert!(keyboard.state().buttons.contains(PadState::L));
        keyboard.aim_by(500.0, 500.0);
        assert_eq!(keyboard.state().touch, Some((319, 239)), "held at the corner, not back in the middle");
        keyboard.set_aim(None);
        assert_eq!(keyboard.state().touch, None);
        assert!(!keyboard.state().buttons.contains(PadState::L));
    }

    #[test]
    fn land_aim_holds_still_then_lifts_then_goes_on_from_the_middle() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.set_aim(Some(AimMode::Land));
        keyboard.stylus(true);
        keyboard.aim_by(0.0, 20.0);
        keyboard.aim_by(150.0, 0.0);
        // still at the edge of its reach, so lifting is no flick
        assert_eq!(keyboard.state().touch, Some((300, 140)));
        keyboard.aim_by(30.0, 0.0);
        for _ in 0..HOLD_FRAMES {
            assert_eq!(keyboard.state().touch, Some((300, 140)), "holding still, the movement waits");
            keyboard.frame_done();
        }
        for _ in 0..LIFT_FRAMES {
            assert_eq!(keyboard.state().touch, None, "lifted");
            keyboard.frame_done();
        }
        assert_eq!(keyboard.state().touch, Some((190, 140)), "down in the middle, on by what came meanwhile, y kept");
        keyboard.aim_by(0.0, 500.0);
        assert_eq!(keyboard.state().touch, Some((190, 239)), "up and down stop at the edge");
    }

    #[test]
    fn land_aim_drags_only_while_the_right_button_holds_the_stylus() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.set_aim(Some(AimMode::Land));
        keyboard.aim_by(30.0, 0.0);
        assert_eq!(keyboard.state().touch, None, "up until the right button");
        keyboard.stylus(true);
        assert_eq!(keyboard.state().touch, Some((160, 120)), "down in the middle");
        keyboard.aim_by(20.0, 10.0);
        assert_eq!(keyboard.state().touch, Some((180, 130)));
        keyboard.stylus(false);
        keyboard.aim_by(40.0, 0.0);
        for _ in 0..HOLD_FRAMES {
            assert_eq!(keyboard.state().touch, Some((180, 130)), "holds still before lifting, no flick");
            keyboard.frame_done();
        }
        for _ in 0..LIFT_FRAMES + 3 {
            assert_eq!(keyboard.state().touch, None, "stays up");
            keyboard.frame_done();
        }
        keyboard.stylus(true);
        assert_eq!(keyboard.state().touch, Some((160, 130)), "down in the middle again, y kept");
        keyboard.stylus(false);
        keyboard.stylus(true);
        keyboard.aim_by(5.0, 0.0);
        assert_eq!(keyboard.state().touch, Some((165, 130)), "pressed again before lifting, the drag goes on");
        keyboard.key(PhysicalKey::Code(KeyCode::Space), true);
        assert!(keyboard.state().buttons.contains(PadState::L), "Space fires");
    }

    #[test]
    fn land_aim_without_the_stylus_saves_the_movement_for_mouse_look() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.set_aim(Some(AimMode::Land));
        keyboard.aim_by(10.0, -4.0);
        keyboard.aim_by(5.0, 1.0);
        assert_eq!(keyboard.state().touch, None, "the stylus stays up");
        assert_eq!(keyboard.take_look(), (15.0, -3.0));
        assert_eq!(keyboard.take_look(), (0.0, 0.0), "taken once");
        keyboard.stylus(true);
        keyboard.aim_by(10.0, 0.0);
        assert_eq!(keyboard.take_look(), (0.0, 0.0), "a held stylus drags instead");
        keyboard.set_aim(Some(AimMode::Air));
        keyboard.aim_by(10.0, 0.0);
        assert_eq!(keyboard.take_look(), (0.0, 0.0), "only on foot");
    }

    #[test]
    fn mouse_look_waits_in_the_mailbox_until_the_game_takes_it() {
        let mut keyboard = Keyboard::new(Keys::default());
        let mut mailbox = [7u32; 4];
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox, [0, 0, 0, 7], "aim off writes nothing, the game's word is left");
        keyboard.set_aim(Some(AimMode::Land));
        keyboard.aim_by(10.0, -2.0);
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox[..3], [MOUSE_LOOK, 10f32.to_bits(), (-2f32).to_bits()]);
        // quick movement, past the easing curve's knee, so the numbers stay
        keyboard.aim_by(AIM_KNEE, 0.0);
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), 10.0 + AIM_KNEE, "not taken yet, so it adds up");
        mailbox[1] = 0;
        mailbox[2] = 0;
        keyboard.aim_by(500.0, 0.0);
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), 500.0, "fast movement is not cut short");
        for _ in 0..STALE_FRAMES {
            keyboard.aim_by(AIM_KNEE, 0.0);
            keyboard.feed_mailbox(&mut mailbox);
        }
        assert_eq!(f32::from_bits(mailbox[1]), 500.0 + AIM_KNEE * STALE_FRAMES as f32, "a game tick can take a few frames");
        keyboard.aim_by(AIM_KNEE, 0.0);
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), AIM_KNEE, "left untaken longer, as in a menu, it is dropped");
        keyboard.set_aim(Some(AimMode::Air));
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(mailbox, [0, 0, 0, 7], "only on foot");
    }

    #[test]
    fn the_game_mode_override_picks_the_aim_mode() {
        let mut keyboard = Keyboard::new(Keys::default());
        assert_eq!(keyboard.game_mode(0), None, "no override, nothing changes");
        assert_eq!(keyboard.game_mode(GAME_ON_FOOT), Some(Some(AimMode::Land)));
        assert_eq!(keyboard.game_mode(0), None, "between ticks");
        assert_eq!(keyboard.game_mode(GAME_ON_FOOT), None, "already on foot, a key the player pressed stands");
        assert_eq!(keyboard.game_mode(GAME_FLYING), Some(Some(AimMode::Air)));
        for _ in 0..QUIET_FRAMES {
            assert_eq!(keyboard.game_mode(0), None);
        }
        assert_eq!(keyboard.game_mode(0), Some(None), "quiet for long, a menu: aim off");
        assert_eq!(keyboard.game_mode(0), None, "said once");
        assert_eq!(keyboard.game_mode(GAME_FLYING), Some(Some(AimMode::Air)), "back in play");
    }

    #[test]
    fn flying_aims_through_the_mailbox_once_the_game_says_it_flies() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.set_aim(Some(AimMode::Air));
        keyboard.aim_by(10.0, 0.0);
        assert_eq!(keyboard.state().touch, Some((170, 120)), "without the game's word, the stylus as before");
        assert_eq!(keyboard.game_mode(GAME_FLYING), Some(Some(AimMode::Air)));
        keyboard.set_aim(Some(AimMode::Air));
        keyboard.aim_by(10.0, -4.0);
        assert_eq!(keyboard.state().touch, None, "the stylus stays up");
        let mut mailbox = [0u32; 4];
        keyboard.feed_mailbox(&mut mailbox);
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
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.game_mode(GAME_FLYING);
        keyboard.set_aim(Some(AimMode::Air));
        keyboard.aim_by(1.0, 0.0);
        let mut mailbox = [0u32; 4];
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), eased(1.0, 0.0).0, "flying feeds eased movement");
        keyboard.set_aim(Some(AimMode::Land));
        keyboard.aim_by(1.0, 0.0);
        keyboard.feed_mailbox(&mut mailbox);
        assert_eq!(f32::from_bits(mailbox[1]), eased(1.0, 0.0).0, "on foot the camera eases the same way");
    }

    #[test]
    fn a_wheel_notch_presses_for_a_few_frames_while_aiming() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.wheel(PadState::RIGHT);
        assert!(!keyboard.state().buttons.contains(PadState::RIGHT), "not without aim");
        keyboard.set_aim(Some(AimMode::Land));
        keyboard.wheel(PadState::RIGHT);
        for _ in 0..WHEEL_FRAMES {
            assert!(keyboard.state().buttons.contains(PadState::RIGHT));
            keyboard.frame_done();
        }
        assert!(!keyboard.state().buttons.contains(PadState::RIGHT));
        keyboard.mouse_button(PadState::DOWN, true);
        assert!(keyboard.state().buttons.contains(PadState::DOWN));
    }

    #[test]
    fn shift_walks() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.key(PhysicalKey::Code(KeyCode::KeyD), true);
        keyboard.key(PhysicalKey::Code(KeyCode::ShiftLeft), true);
        assert_eq!(keyboard.state().circle_x, 0.5);
    }

    #[test]
    fn opposite_circle_keys_cancel_out() {
        let mut keyboard = Keyboard::new(Keys::default());
        keyboard.key(PhysicalKey::Code(KeyCode::KeyA), true);
        assert_eq!(keyboard.state().circle_x, -1.0);
        keyboard.key(PhysicalKey::Code(KeyCode::KeyD), true);
        assert_eq!(keyboard.state().circle_x, 0.0);
        keyboard.release();
        assert_eq!(keyboard.state().circle_x, 0.0);
    }
}
