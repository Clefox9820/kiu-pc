//! the time the GPU's emulation spends in a frame, by what it is on, so a
//! slow frame can say where its time went.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// nanoseconds, since the last take.
pub static DRAWING: AtomicU64 = AtomicU64::new(0);
pub static SHADING: AtomicU64 = AtomicU64::new(0);
pub static TEXTURES: AtomicU64 = AtomicU64::new(0);
pub static WAITING: AtomicU64 = AtomicU64::new(0);
pub static DRAWS: AtomicU64 = AtomicU64::new(0);
/// the rest of the console's frame, which zakuro-core times: its recompiled
/// code, its services and syscalls, the end of the frame and the sound.
pub static RECOMPILED: AtomicU64 = AtomicU64::new(0);
pub static SERVICES: AtomicU64 = AtomicU64::new(0);
pub static FRAME_END: AtomicU64 = AtomicU64::new(0);
pub static AUDIO: AtomicU64 = AtomicU64::new(0);
/// what the GPU drew brought back for the CPU's reads and writes, and how often.
pub static SYNC: AtomicU64 = AtomicU64::new(0);
pub static SYNCS: AtomicU64 = AtomicU64::new(0);

/// what a frame spent, in milliseconds, and the draws it made.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spent {
    pub drawing: f64,
    pub shading: f64,
    pub textures: f64,
    pub waiting: f64,
    pub draws: u64,
    pub recompiled: f64,
    pub services: f64,
    pub frame_end: f64,
    pub audio: f64,
    pub sync: f64,
    pub syncs: u64,
}

/// what was spent since the last take, starting afresh.
pub fn take() -> Spent {
    let ms = |counter: &AtomicU64| counter.swap(0, Ordering::Relaxed) as f64 / 1e6;
    Spent {
        drawing: ms(&DRAWING),
        shading: ms(&SHADING),
        textures: ms(&TEXTURES),
        waiting: ms(&WAITING),
        draws: DRAWS.swap(0, Ordering::Relaxed),
        recompiled: ms(&RECOMPILED),
        services: ms(&SERVICES),
        frame_end: ms(&FRAME_END),
        audio: ms(&AUDIO),
        sync: ms(&SYNC),
        syncs: SYNCS.swap(0, Ordering::Relaxed),
    }
}

/// adds the time until it is dropped to a counter.
pub struct Timer(&'static AtomicU64, Instant);

pub fn time(counter: &'static AtomicU64) -> Timer {
    Timer(counter, Instant::now())
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.0.fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timer_adds_to_its_counter_until_taken() {
        take();
        {
            let _timer = time(&SHADING);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(take().shading >= 2.0);
        assert_eq!(take().shading, 0.0);
    }
}
