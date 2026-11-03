//! with ZAKURO_WAV=<file.wav>, the console's sound from ZAKURO_WAV_FROM
//! seconds of it on (5 by default), for ZAKURO_WAV_SECONDS (8), turned into
//! 48 kHz as the output does, away from the device's timing and whatever
//! the volume, and saved once there is enough. a save state played with no
//! input makes the same sound every run, so two builds' files compare by
//! ear. a part of audio, whose queue it plays through.

use super::Queue;
use crate::settings::Latency;

/// the rate captured sound is saved at.
const CAPTURE_RATE: u32 = 48_000;

/// the sound being saved.
pub(super) struct Capture {
    path: std::path::PathBuf,
    /// the console's samples still to pass over before it starts.
    skip: usize,
    queue: Queue,
    out: Vec<[f32; 2]>,
    /// how many output samples it saves.
    wanted: usize,
}

impl Capture {
    /// the capture ZAKURO_WAV asks for, of sound made at `rate` samples a
    /// second, when it is set.
    pub(super) fn from_env(rate: f64) -> Option<Capture> {
        let asked = crate::dev_env::wav_capture()?;
        let step = rate / CAPTURE_RATE as f64;
        let mut queue = Queue::new(step, Latency::Normal.samples());
        queue.starved = false;
        queue.level = 1.0;
        Some(Capture {
            path: asked.path,
            skip: (asked.from * rate) as usize,
            queue,
            out: Vec::new(),
            wanted: (asked.seconds * CAPTURE_RATE as f64) as usize,
        })
    }

    /// takes in what the console played, and says once it saved the file.
    pub(super) fn feed(&mut self, samples: &[[i16; 2]]) -> bool {
        let skipped = self.skip.min(samples.len());
        self.skip -= skipped;
        self.queue.samples.extend(samples[skipped..].iter().map(|s| s.map(|v| v as f32 / 32768.0)));
        // played while the queue is at its target, so DRIFT leaves the pitch alone
        while self.queue.samples.len() > self.queue.target && self.out.len() < self.wanted {
            self.out.push(self.queue.next());
        }
        if self.out.len() < self.wanted {
            return false;
        }
        match write_wav(&self.path, &self.out) {
            Ok(()) => log::info!("sound captured to {}", self.path.display()),
            Err(error) => log::warn!("could not save the sound to {}, {error}", self.path.display()),
        }
        true
    }
}

/// saves stereo samples as a 16-bit WAV file at CAPTURE_RATE.
fn write_wav(path: &std::path::Path, samples: &[[f32; 2]]) -> std::io::Result<()> {
    let data = samples.len() as u32 * 4;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    // PCM, two channels, the rate, bytes a second, bytes a frame, bits
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&CAPTURE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(CAPTURE_RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples.iter().flatten() {
        bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a capture passes over the start it was told to, then saves as much
    /// as it was asked for as a 48 kHz stereo WAV file.
    #[test]
    fn a_capture_saves_a_wav_of_the_sound_asked_for() {
        let path = std::env::temp_dir().join(format!("zakuro-capture-{}.wav", std::process::id()));
        let mut queue = Queue::new(32728.0 / 48000.0, Latency::Normal.samples());
        queue.starved = false;
        queue.level = 1.0;
        let mut capture = Capture { path: path.clone(), skip: 1000, queue, out: Vec::new(), wanted: 4800 };
        let sound: Vec<[i16; 2]> = (0..20_000).map(|i| [(i % 100) as i16 * 100; 2]).collect();
        assert!(sound.chunks(547).any(|chunk| capture.feed(chunk)), "it saves once there is enough");
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(bytes.len(), 44 + 4800 * 4);
        assert_eq!((&bytes[..4], &bytes[8..16], &bytes[36..40]), (&b"RIFF"[..], &b"WAVEfmt "[..], &b"data"[..]));
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), CAPTURE_RATE);
    }
}
