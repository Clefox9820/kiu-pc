//! what Kid Icarus Uprising is doing, as its game overrides tell the host
//! through the mailbox's game mode word: in play or in a menu, flying or on
//! foot, and whether a video plays; and what the window shows for that.

use zakuro_gpu::{ScreenLayout, Viewport};

use crate::settings::TopShape;

/// the game mode word's values, which game overrides set every tick to the
/// kind of play going on.
pub const GAME_ON_FOOT: u32 = 1;
pub const GAME_FLYING: u32 = 2;

/// frames without a game override saying the game is in play before it is
/// taken to be in a menu. the overrides say it once a tick, every second
/// frame for Kid Icarus.
pub const IN_PLAY_QUIET: u32 = 10;

/// frames after the last frame a video's decoder converted before the top
/// screen goes back to its own shape, a video's frames coming at 30 a
/// second or fewer.
const VIDEO_HOLD: u64 = 30;

/// what the game is doing, from frame to frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayState {
    /// frames since a game override last said the title is in play.
    out_of_play: u32,
    /// the game override last said the player is flying, not on foot.
    flying: bool,
    /// how many video frames the decoder had converted at the last frame,
    /// and the frame a new one came at, see VIDEO_HOLD.
    conversions: u64,
    video_at: Option<u64>,
}

impl Default for PlayState {
    /// a game starts out of play, as in its menus.
    fn default() -> PlayState {
        PlayState { out_of_play: u32::MAX, flying: false, conversions: 0, video_at: None }
    }
}

impl PlayState {
    /// takes a frame's game mode word, 0 when the overrides said nothing,
    /// and says whether the game went into play or out of it.
    pub fn update(&mut self, said: u32) -> bool {
        let was_in_play = self.in_play();
        self.out_of_play = if said != 0 { 0 } else { self.out_of_play.saturating_add(1) };
        if said != 0 {
            self.flying = said == GAME_FLYING;
        }
        self.in_play() != was_in_play
    }

    /// in play while the overrides say so, see IN_PLAY_QUIET.
    pub fn in_play(&self) -> bool {
        self.out_of_play <= IN_PLAY_QUIET
    }

    /// the player flies, as the game last said, rather than walks.
    pub fn flying(&self) -> bool {
        self.flying
    }

    /// takes how many video frames the decoder had converted by this frame,
    /// and says whether a video plays, see VIDEO_HOLD.
    pub fn video(&mut self, conversions: u64, frame: u64) -> bool {
        if conversions != self.conversions {
            self.conversions = conversions;
            self.video_at = Some(frame);
        }
        self.video_at.is_some_and(|at| frame.saturating_sub(at) <= VIDEO_HOLD)
    }
}

/// how wide the top screen shows: in its chosen shape, but the console's
/// while a video plays, a picture made for 400 across that cannot be
/// widened as the 3D is, and in menus on a 21:9 screen, whose 3D would
/// show what lies off the console's view there.
pub fn top_width_shown(shape: TopShape, video: bool, in_play: bool) -> f32 {
    let console = video || (!in_play && shape == TopShape::Wide21x9);
    if console { TopShape::Console.width() } else { shape.width() }
}

/// whether a layout shows the top screen and the bottom one.
pub fn shows(layout: ScreenLayout) -> (bool, bool) {
    match layout {
        ScreenLayout::TopOnly => (true, false),
        ScreenLayout::BottomOnly => (false, true),
        _ => (true, true),
    }
}

/// the window's rectangles no screen covers, beside, above and below each
/// screen as far as the next screen or the window's edge, in window pixels, as
/// [x0, y0, x1, y1].
pub fn free_regions(width: f32, height: f32, screens: &[Viewport]) -> Vec<[f32; 4]> {
    let mut free = Vec::new();
    for s in screens {
        let (x0, y0, x1, y1) = (s.x, s.y, s.x + s.width, s.y + s.height);
        let in_rows = |o: &&Viewport| o.y < y1 && o.y + o.height > y0;
        let in_columns = |o: &&Viewport| o.x < x1 && o.x + o.width > x0;
        let left = screens.iter().filter(in_rows).map(|o| o.x + o.width).filter(|&e| e <= x0).fold(0.0, f32::max);
        let right = screens.iter().filter(in_rows).map(|o| o.x).filter(|&e| e >= x1).fold(width, f32::min);
        let above = screens.iter().filter(in_columns).map(|o| o.y + o.height).filter(|&e| e <= y0).fold(0.0, f32::max);
        let below = screens.iter().filter(in_columns).map(|o| o.y).filter(|&e| e >= y1).fold(height, f32::min);
        free.extend([[left, y0, x0, y1], [x1, y0, right, y1], [x0, above, x1, y0], [x0, y1, x1, below]]);
    }
    free.retain(|[x0, y0, x1, y1]| x1 - x0 >= 1.0 && y1 - y0 >= 1.0);
    free
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_is_in_play_while_the_overrides_say_so_and_a_while_after() {
        let mut play = PlayState::default();
        assert!(!play.in_play(), "a game starts in its menus");
        assert!(!play.update(0), "nothing said, nothing changes");
        assert!(play.update(GAME_ON_FOOT), "into play");
        assert!(play.in_play() && !play.flying());
        for _ in 0..IN_PLAY_QUIET {
            assert!(!play.update(0), "between ticks it stays in play");
        }
        assert!(play.update(0), "quiet for long, a menu");
        assert!(!play.in_play());
        assert!(play.update(GAME_FLYING));
        assert!(play.flying());
        play.update(0);
        assert!(play.flying(), "flying until the game says otherwise");
    }

    #[test]
    fn a_video_plays_while_the_decoder_converts_and_a_moment_after() {
        let mut play = PlayState::default();
        assert!(!play.video(0, 1), "nothing converted");
        assert!(play.video(1, 2), "a frame converted");
        assert!(play.video(1, 2 + VIDEO_HOLD), "held a while");
        assert!(!play.video(1, 3 + VIDEO_HOLD), "then over");
        assert!(play.video(2, 4 + VIDEO_HOLD), "the next video");
    }

    /// the layouts with one screen hide the other one, the rest show both,
    /// the bottom screen over the top one's included.
    #[test]
    fn a_layout_with_one_screen_hides_the_other() {
        use ScreenLayout::*;
        assert_eq!(shows(TopOnly), (true, false));
        assert_eq!(shows(BottomOnly), (false, true));
        for layout in [Stacked, SideBySide, LargeTop, BottomOverTop] {
            assert_eq!(shows(layout), (true, true), "{layout:?}");
        }
    }

    fn viewport(x: f32, y: f32, width: f32, height: f32) -> Viewport {
        Viewport { x, y, width, height }
    }

    #[test]
    fn free_regions_are_the_margins_and_gaps_beside_the_screens() {
        // stacked in a wide window: margins left and right, wider beside the
        // narrower bottom screen
        let free = free_regions(1000.0, 960.0, &[viewport(100.0, 0.0, 800.0, 480.0), viewport(180.0, 480.0, 640.0, 480.0)]);
        assert!(free.contains(&[0.0, 0.0, 100.0, 480.0]));
        assert!(free.contains(&[0.0, 480.0, 180.0, 960.0]));
        assert!(free.contains(&[820.0, 480.0, 1000.0, 960.0]));
        // large top: the block above the small bottom screen, beside the top
        let free = free_regions(1120.0, 480.0, &[viewport(0.0, 0.0, 800.0, 480.0), viewport(800.0, 240.0, 320.0, 240.0)]);
        assert!(free.contains(&[800.0, 0.0, 1120.0, 240.0]));
        assert!(!free.iter().any(|[x0, _, _, _]| *x0 < 800.0), "nothing over the top screen");
    }

    /// the top screen takes its chosen shape, but the console's while a
    /// video plays, which cannot be widened, and in menus on a 21:9 screen,
    /// whose 3D shows what lies off the console's view there.
    #[test]
    fn the_top_screen_keeps_the_consoles_shape_for_videos_and_wide_menus() {
        use TopShape::*;
        assert_eq!(top_width_shown(Wide16x9, false, true), Wide16x9.width(), "in play");
        assert_eq!(top_width_shown(Wide16x9, true, true), 400.0, "a video");
        assert_eq!(top_width_shown(Wide16x9, false, false), Wide16x9.width(), "16:9 menus stay wide");
        assert_eq!(top_width_shown(Wide21x9, false, false), 400.0, "21:9 menus");
        assert_eq!(top_width_shown(Wide21x9, true, false), 400.0, "a video in a 21:9 menu");
        assert_eq!(top_width_shown(Wide21x9, false, true), Wide21x9.width(), "21:9 in play");
        assert_eq!(top_width_shown(Console, false, true), 400.0, "the console's shape");
    }
}
