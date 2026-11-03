//! the environment switches for test and comparison runs, read in one place:
//!
//! - `ZAKURO_SHOT=<path prefix>` saves both screens once a pacing window, see
//!   frame_log.
//! - `ZAKURO_LOAD_SLOT=<1-9>` goes straight on from that slot's save state
//!   when a game starts, for runs nobody has to steer to the same spot.
//! - `ZAKURO_WAV=<file.wav>` saves the console's sound from
//!   `ZAKURO_WAV_FROM` seconds of it on (5 by default), for
//!   `ZAKURO_WAV_SECONDS` (8), see audio_capture.
//! - `ZAKURO_NO_WARM=1` leaves the code and the ROM to the disk, see warm.

use std::path::PathBuf;

/// where ZAKURO_SHOT saves the screens, the start of their files' paths.
pub fn shot_prefix() -> Option<String> {
    std::env::var("ZAKURO_SHOT").ok()
}

/// the save state slot ZAKURO_LOAD_SLOT goes on from, 1 to 9.
pub fn load_slot() -> Option<u8> {
    std::env::var("ZAKURO_LOAD_SLOT").ok().and_then(|slot| slot.parse().ok()).filter(|slot| (1..=9).contains(slot))
}

/// ZAKURO_NO_WARM is set, to compare warming with the disk alone.
pub fn no_warm() -> bool {
    std::env::var_os("ZAKURO_NO_WARM").is_some()
}

/// the sound ZAKURO_WAV asks to save: where, and which seconds of it.
#[derive(Debug, PartialEq)]
pub struct WavCapture {
    pub path: PathBuf,
    pub from: f64,
    pub seconds: f64,
}

/// what ZAKURO_WAV and its two companions ask for, when it is set.
pub fn wav_capture() -> Option<WavCapture> {
    let path = std::env::var_os("ZAKURO_WAV")?.into();
    let read = |name: &str, default: f64| seconds(name, std::env::var(name).ok().as_deref(), default);
    Some(WavCapture { path, from: read("ZAKURO_WAV_FROM", 5.0), seconds: read("ZAKURO_WAV_SECONDS", 8.0) })
}

/// a number of seconds from the variable `name`'s value, or the default when
/// it is unset or no number of seconds, a negative one included, which would
/// otherwise save an empty file at once.
fn seconds(name: &str, value: Option<&str>, default: f64) -> f64 {
    let Some(value) = value else { return default };
    match value.parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => seconds,
        _ => {
            log::warn!("{name}={value} is no number of seconds, taking {default}");
            default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_that_cannot_be_take_the_default() {
        assert_eq!(seconds("ZAKURO_WAV_SECONDS", Some("2.5"), 8.0), 2.5);
        assert_eq!(seconds("ZAKURO_WAV_SECONDS", Some("0"), 8.0), 0.0, "none asked for");
        assert_eq!(seconds("ZAKURO_WAV_SECONDS", None, 8.0), 8.0);
        assert_eq!(seconds("ZAKURO_WAV_SECONDS", Some("-3"), 8.0), 8.0, "a negative length");
        assert_eq!(seconds("ZAKURO_WAV_SECONDS", Some("eight"), 8.0), 8.0);
        assert_eq!(seconds("ZAKURO_WAV_SECONDS", Some("inf"), 8.0), 8.0);
    }
}
