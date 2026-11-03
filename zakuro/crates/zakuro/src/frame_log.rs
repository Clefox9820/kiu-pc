//! what each frame of the game cost, for the log: every frame's times with
//! RUST_LOG=zakuro::frames=debug, how evenly the frames came once a window
//! of them, and a warning for each hitch the player feels, with what the
//! game was doing in it.

use std::time::{Duration, Instant};

use zakuro_core::System;

/// with ZAKURO_SHOT=<path prefix>, saves both screens as the GPU shows them
/// to <prefix>-<frame>-top.png and -bottom.png, once a pacing window, for
/// checking a change to the renderer by eye where the window can't be
/// captured. frame interpolation's pictures in between go to
/// -top-between.png and -bottom-between.png, when it made them, to compare
/// them between builds pixel by pixel.
fn save_screens(system: &mut System, frame: u64) {
    static PREFIX: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(prefix) = PREFIX.get_or_init(|| std::env::var("ZAKURO_SHOT").ok()) else { return };
    for (screen, name) in [(zakuro_common::Screen::Top, "top"), (zakuro_common::Screen::Bottom, "bottom")] {
        for (between, suffix) in [(false, ""), (true, "-between")] {
            let Some((pixels, width, height)) = system.upright_screen(screen, between) else { continue };
            let path = format!("{prefix}-{frame}-{name}{suffix}.png");
            if let Err(error) = image::save_buffer(&path, &pixels, width, height, image::ExtendedColorType::Rgba8) {
                log::warn!("could not save {path}, {error}");
            }
        }
    }
}

/// how many frames of a window came late, the small stutters a hitch line
/// is too coarse for: one over 20 ms shows a frame twice, one over 33 ms
/// shows it three times or more.
#[derive(Debug, Default, PartialEq)]
struct Pacing {
    frames: u32,
    late: u32,
    very_late: u32,
    worst: Duration,
}

impl Pacing {
    /// frames counted before the log hears of them, 10 s at 60 a second.
    const WINDOW: u32 = 600;

    /// counts a frame that came the time given after the one before; the
    /// window's counts, once it is full.
    fn note(&mut self, since: Duration) -> Option<Pacing> {
        self.frames += 1;
        self.late += (since > Duration::from_millis(20)) as u32;
        self.very_late += (since > Duration::from_millis(33)) as u32;
        self.worst = self.worst.max(since);
        (self.frames >= Self::WINDOW).then(|| std::mem::take(self))
    }
}

/// what the game had run when a frame started.
struct Start {
    at: Instant,
    interpreted: u64,
    faults: u64,
}

/// watches a game's frames for the log.
#[derive(Default)]
pub struct FrameProbe {
    /// how evenly the frames came.
    pacing: Pacing,
    /// when the last frame was emulated, to tell hitches from the frames' spacing.
    last_emulated: Option<Instant>,
    start: Option<Start>,
}

/// the instructions interpreted so far, the recompiled code's fallbacks
/// among them.
fn interpreted(system: &System) -> u64 {
    system.interpreted_instructions + system.recompiled.as_ref().map_or(0, |library| library.fallbacks())
}

impl FrameProbe {
    /// a pause is no hitch, the next frame's spacing starts afresh.
    pub fn pause(&mut self) {
        self.last_emulated = None;
    }

    /// notes what the game has run so far, right before its frame runs.
    pub fn start(&mut self, system: &System) {
        self.start = Some(Start { interpreted: interpreted(system), faults: crate::warm::page_faults(), at: Instant::now() });
    }

    /// logs the frame just run, number `frame`: its times, the pacing once a
    /// window is full, and a hitch. a pause or a menu (`menu`) is no hitch,
    /// and fast forward (`fast`) runs frames as fast as it can, no pacing to
    /// speak of.
    pub fn end(&mut self, system: &mut System, frame: u64, menu: bool, fast: bool) {
        let Some(start) = self.start.take() else { return };
        // a frame far over its 16.7 ms is a hitch the player feels; the log
        // around it says what the game was doing
        let took = start.at.elapsed();
        let gpu = zakuro_gpu::stats::take();
        // and the whole time since the last frame, showing it included, which
        // catches a stall outside the emulation too
        let since = self.last_emulated.replace(start.at).map(|last| start.at - last).unwrap_or_default();
        // every frame, with RUST_LOG=zakuro::frames=debug, to see a pattern
        // the window's counts only add up
        log::debug!(
            target: "zakuro::frames",
            "frame {} took {:.1} ms, {:.1} ms since the last, {} draws in {:.1} ms, waiting for the GPU {:.1} ms",
            frame,
            took.as_secs_f64() * 1000.0,
            since.as_secs_f64() * 1000.0,
            gpu.draws,
            gpu.drawing,
            gpu.waiting
        );
        if !menu && !fast && !since.is_zero() {
            if let Some(window) = self.pacing.note(since) {
                log::info!(
                    "pacing: {} frames, {} over 20 ms, {} over 33 ms, the worst {:.0} ms",
                    window.frames,
                    window.late,
                    window.very_late,
                    window.worst.as_secs_f64() * 1000.0
                );
                save_screens(system, frame);
            }
        }
        if !menu && (took.as_millis() >= 50 || since.as_millis() >= 100) {
            let work = Work { interpreted: interpreted(system) - start.interpreted, pages: crate::warm::page_faults() - start.faults };
            log::warn!("{}", hitch(frame, took, since, work, &gpu));
        }
    }
}

/// what a frame did besides drawing: instructions interpreted and pages the
/// system brought in.
struct Work {
    interpreted: u64,
    pages: u64,
}

/// the line a hitch writes to the log.
fn hitch(frame: u64, took: Duration, since: Duration, work: Work, gpu: &zakuro_gpu::stats::Spent) -> String {
    format!(
        "hitch: frame {} took {:.0} ms, {:.0} ms since the last, {} instructions interpreted, \
         {} draws in {:.0} ms: vertices {:.0} ms, textures {:.0} ms, waiting for the GPU {:.0} ms; \
         recompiled code {:.0} ms, services {:.0} ms, frame end {:.0} ms, sound {:.0} ms, \
         {} syncs with the GPU in {:.0} ms, {} pages brought in",
        frame,
        took.as_secs_f64() * 1000.0,
        since.as_secs_f64() * 1000.0,
        work.interpreted,
        gpu.draws,
        gpu.drawing,
        gpu.shading,
        gpu.textures,
        gpu.waiting,
        gpu.recompiled,
        gpu.services,
        gpu.frame_end,
        gpu.audio,
        gpu.syncs,
        gpu.sync,
        work.pages
    )
}

/// with RUST_LOG=zakuro::frames=debug, how a frame met the display: how
/// long showing it took (a presenter waiting for the display blocks there),
/// or that it went unshown, and the sleep after.
pub fn shown(presented: Option<Duration>, sleep: Duration) {
    match presented {
        Some(took) => log::debug!(
            target: "zakuro::frames",
            "shown in {:.1} ms, then slept {:.1} ms",
            took.as_secs_f64() * 1000.0,
            sleep.as_secs_f64() * 1000.0
        ),
        None => log::debug!(target: "zakuro::frames", "unshown, behind, slept {:.1} ms", sleep.as_secs_f64() * 1000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_counts_late_frames_once_a_window() {
        let mut pacing = Pacing::default();
        let ms = Duration::from_millis;
        for frame in 1..Pacing::WINDOW {
            let since = match frame {
                10 => ms(25),
                20 => ms(40),
                _ => ms(16),
            };
            assert_eq!(pacing.note(since), None, "frame {frame} is before the window is full");
        }
        let window = pacing.note(ms(21)).expect("the window is full");
        assert_eq!(window, Pacing { frames: Pacing::WINDOW, late: 3, very_late: 1, worst: ms(40) });
        // the next window starts from nothing
        assert_eq!(pacing, Pacing::default());
    }

    #[test]
    fn a_hitch_reads_as_one_line() {
        let gpu = zakuro_gpu::stats::Spent { draws: 120, syncs: 2, ..Default::default() };
        let line = hitch(42, Duration::from_millis(80), Duration::from_millis(96), Work { interpreted: 7, pages: 3 }, &gpu);
        assert!(!line.contains("  "), "no padding in the middle: {line}");
        assert!(line.starts_with("hitch: frame 42 took 80 ms, 96 ms since the last, 7 instructions interpreted, 120 draws"), "{line}");
        assert!(line.ends_with("2 syncs with the GPU in 0 ms, 3 pages brought in"), "{line}");
    }
}
