//! the console's sound, played on the host's chosen or default output
//! device.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

#[path = "audio_capture.rs"]
mod capture;

use capture::Capture;

/// the queue's target at the latency sound had before it could be chosen,
/// about 125 ms; Queue::target holds what the settings chose.
#[cfg(test)]
const TARGET: usize = crate::settings::Latency::Normal.samples();
#[cfg(test)]
const RESUME: usize = TARGET / 4;
/// how much faster or slower than the rates say playback may go to keep
/// the queue near its target, too little to hear.
const DRIFT: f64 = 0.005;
/// how much of its level dry output keeps from one sample to the next, about
/// 20 ms to fall to a third at 48 kHz.
const FADE_OUT: f32 = 0.999;
/// how many output samples sound takes to climb back to full level after
/// running dry, about 10 ms at 48 kHz.
const FADE_IN: f32 = 480.0;

/// how many of the console's samples each output sample is made from, half
/// before it and half after.
const TAPS: usize = 16;
/// the queue's sample just before the output's place, the ones ahead of it
/// being history the filter still reads.
const CENTRE: usize = TAPS / 2 - 1;
/// how many places between two of the console's samples the filter is
/// worked out for; the nearest is used.
const PHASES: usize = 512;

/// a windowed-sinc low-pass filter for turning the console's rate into the
/// output's: for each place between two samples, the weights of the TAPS
/// samples around it. it keeps what lies below the lower of the two rates'
/// halves, with a little room for the filter to fall off, and drops what
/// would fold back as false tones. each set of weights adds up to one, so
/// a steady level stays the same.
fn sinc_filter(step: f64) -> Vec<[f32; TAPS]> {
    // as a part of the console's half rate, the output's when lower
    let cutoff = (1.0 / step).min(1.0) * 0.9;
    (0..PHASES)
        .map(|phase| {
            let fraction = phase as f64 / PHASES as f64;
            let mut weights = [0.0f32; TAPS];
            for (k, weight) in weights.iter_mut().enumerate() {
                // how far the sample is from the output's place
                let x = k as f64 - CENTRE as f64 - fraction;
                let angle = std::f64::consts::PI * cutoff * x;
                let sinc = if x == 0.0 { 1.0 } else { angle.sin() / angle };
                // a Blackman window across the taps, 0 to 1
                let n = (x + TAPS as f64 / 2.0) / TAPS as f64;
                let tau = std::f64::consts::TAU;
                let window = 0.42 - 0.5 * (tau * n).cos() + 0.08 * (2.0 * tau * n).cos();
                *weight = (sinc * window) as f32;
            }
            let sum: f32 = weights.iter().sum();
            weights.map(|w| w / sum)
        })
        .collect()
}

/// the console's samples waiting to be played.
struct Queue {
    samples: VecDeque<[f32; 2]>,
    /// how far the output is between samples CENTRE and CENTRE + 1.
    fraction: f64,
    /// console samples per output sample.
    step: f64,
    /// sinc_filter for step.
    filter: Vec<[f32; TAPS]>,
    /// ran dry, and waits to fill up before it plays again.
    starved: bool,
    /// the last sample filtered, which fades out when the queue runs dry
    /// instead of stopping short.
    last: [f32; 2],
    /// how loud what comes out is, 0 to 1: it falls slowly while the queue
    /// is dry and climbs back over FADE_IN when it plays again, so neither
    /// end of a shortfall clicks.
    level: f32,
    /// times the queue ran dry.
    underruns: u64,
    /// how much a queue that ran dry waits for before it plays again.
    refill: usize,
    /// how much sound to keep waiting, in the console's samples: more
    /// rides out uneven frames, less answers sooner.
    target: usize,
}

impl Queue {
    /// an empty queue playing step console samples per output sample and
    /// keeping target waiting, which fills up before it plays.
    fn new(step: f64, target: usize) -> Queue {
        let mut queue = Queue {
            samples: VecDeque::new(),
            fraction: 0.0,
            step,
            filter: sinc_filter(step),
            starved: true,
            last: [0.0; 2],
            level: 0.0,
            underruns: 0,
            refill: 0,
            target,
        };
        queue.refill = queue.resume_at();
        queue
    }

    /// how much a queue that ran dry waits for before it plays again, a
    /// quarter of its target.
    fn resume_at(&self) -> usize {
        self.target / 4
    }

    /// how far the queue runs down before sound is stretched, half of its
    /// target: every stretch splices the sound, so smaller shortfalls are
    /// left to DRIFT.
    fn stretch_below(&self) -> usize {
        self.target / 2
    }

    /// the next output sample, filtered from the console's around it.
    fn next(&mut self) -> [f32; 2] {
        if !self.starved && self.samples.len() < TAPS {
            self.starved = true;
            self.underruns += 1;
        }
        if self.starved {
            if self.samples.len() < self.refill {
                self.level *= FADE_OUT;
                return self.last.map(|v| v * self.level);
            }
            self.starved = false;
            self.refill = self.resume_at();
        }
        let weights = &self.filter[((self.fraction * PHASES as f64) as usize).min(PHASES - 1)];
        let mut out = [0.0f32; 2];
        for (sample, weight) in self.samples.iter().zip(weights) {
            out[0] += sample[0] * weight;
            out[1] += sample[1] * weight;
        }
        self.last = out;
        self.level = (self.level + 1.0 / FADE_IN).min(1.0);
        let out = out.map(|v| v * self.level);
        // a longer queue plays a little faster, a shorter one slower
        let error = (self.samples.len() as f64 - self.target as f64) / self.target as f64;
        self.fraction += self.step * (1.0 + error.clamp(-1.0, 1.0) * DRIFT);
        while self.fraction >= 1.0 && self.samples.len() >= TAPS {
            self.samples.pop_front();
            self.fraction -= 1.0;
        }
        out
    }

    /// stops it while the game is stopped, without counting that as running
    /// dry, and has it fill all the way up before it plays again, so the
    /// first frames after do not run it dry either.
    fn hold(&mut self) {
        self.starved = true;
        self.refill = self.target;
        self.underruns = 0;
    }

    /// drops what is queued, holding as above, fading out from what played
    /// last.
    fn clear(&mut self) {
        self.samples.clear();
        self.fraction = 0.0;
        self.hold();
    }
}

/// the pieces of sound stretching moves around, about 16 ms, and how far
/// apart they go out, half of that.
const GRAIN: usize = 512;
const HOP: usize = GRAIN / 2;
/// how far a piece may shift, about 4 ms, to line up with the one before.
const SEARCH: usize = 128;
/// sound gets at most twice as long.
const MOST_STRETCH: f64 = 2.0;

/// time stretching, sound made longer without its pitch changing, for when
/// the emulation falls behind and the queue would run dry. overlapping
/// pieces of it go out further apart than they came in, each shifted to
/// line up with the one before, which is WSOLA.
struct Stretch {
    /// what came in and has not gone out yet.
    input: Vec<[f32; 2]>,
    /// where in input the next piece is due.
    position: f64,
    /// where the last piece came from, none before the first.
    last: Option<usize>,
    /// the last piece's second half, which the next one's first adds to.
    tail: Vec<[f32; 2]>,
    /// how much longer sound comes out, eased toward what the queue asks.
    factor: f64,
    window: Vec<f32>,
}

impl Stretch {
    fn new() -> Stretch {
        // a periodic Hann window, whose halves add up to one
        let window = (0..GRAIN).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / GRAIN as f32).cos()).collect();
        Stretch { input: Vec::new(), position: 0.0, last: None, tail: vec![[0.0; 2]; HOP], factor: 1.0, window }
    }

    /// forgets what came in, to start over on other sound.
    fn clear(&mut self) {
        self.input.clear();
        self.position = 0.0;
        self.last = None;
        self.tail.fill([0.0; 2]);
        self.factor = 1.0;
    }

    /// stretches samples by about factor, passing out what is ready.
    fn process(&mut self, samples: &[[f32; 2]], factor: f64, out: &mut Vec<[f32; 2]>) {
        self.input.extend_from_slice(samples);
        self.factor += (factor - self.factor) * 0.2;
        // close to not stretching any more, it stops
        if factor == 1.0 && self.factor < 1.001 {
            self.factor = 1.0;
        }
        let mono = |s: [f32; 2]| s[0] + s[1];
        loop {
            // not stretching, each piece follows on from the last, with no
            // search and so no splice
            if let Some(last) = self.last.filter(|_| self.factor == 1.0) {
                self.position = (last + HOP) as f64;
            }
            let due = self.position as usize;
            let natural = self.last.map_or(due, |last| last + HOP);
            if due + SEARCH + GRAIN > self.input.len() || natural + GRAIN > self.input.len() {
                break;
            }
            // the piece around where it is due that most looks like the
            // last one's continuation, which is where it is due when
            // nothing is stretched
            let start = if natural == due {
                due
            } else {
                let template = &self.input[natural..natural + HOP];
                let score = |candidate: usize| {
                    let (mut dot, mut energy) = (0.0f32, 1e-9f32);
                    for (a, &b) in self.input[candidate..candidate + HOP].iter().zip(template) {
                        dot += mono(*a) * mono(b);
                        energy += mono(*a) * mono(*a);
                    }
                    dot / energy.sqrt()
                };
                (due.saturating_sub(SEARCH)..=due + SEARCH)
                    .map(|candidate| (score(candidate), candidate))
                    // a tie, as in silence, goes to the piece nearest the
                    // natural one
                    .max_by(|(a, x), (b, y)| a.total_cmp(b).then(natural.abs_diff(*y).cmp(&natural.abs_diff(*x))))
                    .map_or(due, |(_, candidate)| candidate)
            };
            let piece = &self.input[start..start + GRAIN];
            let (first, second) = piece.split_at(HOP);
            let (rising, falling) = self.window.split_at(HOP);
            for ((tail, sample), w) in self.tail.iter_mut().zip(first).zip(rising) {
                out.push([tail[0] + sample[0] * w, tail[1] + sample[1] * w]);
            }
            for ((tail, sample), w) in self.tail.iter_mut().zip(second).zip(falling) {
                *tail = [sample[0] * w, sample[1] * w];
            }
            self.last = Some(start);
            self.position += HOP as f64 / self.factor;
        }
        // what no piece can reach any more goes
        let used = (self.position as usize).min(self.last.unwrap_or(0)).saturating_sub(SEARCH);
        if used > 0 {
            self.input.drain(..used);
            self.position -= used as f64;
            self.last = self.last.map(|last| last - used);
        }
    }
}

pub struct Audio {
    queue: Arc<Mutex<Queue>>,
    /// 0 to 1, applied as samples come in.
    volume: std::cell::Cell<f32>,
    stretch: std::cell::RefCell<Stretch>,
    /// with ZAKURO_WAV, the sound being saved, see Capture.
    capture: std::cell::RefCell<Option<Capture>>,
    _stream: cpal::Stream,
}

impl Audio {
    /// opens the output named device, or the default one when it is empty
    /// or not there, for sound made at rate samples a second, keeping target
    /// of them waiting.
    pub fn open(rate: f64, target: usize, device: &str) -> Result<Audio, String> {
        let host = cpal::default_host();
        let chosen = (!device.is_empty())
            .then(|| host.output_devices().ok()?.find(|found| found.name().is_ok_and(|name| name == device)))
            .flatten();
        if chosen.is_none() && !device.is_empty() {
            log::warn!("no sound output named {device}, playing on the default one");
        }
        let device = match chosen {
            Some(device) => device,
            None => host.default_output_device().ok_or("there is no sound output device")?,
        };
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let step = rate / config.sample_rate.0 as f64;
        let queue = Arc::new(Mutex::new(Queue::new(step, target)));
        let stream = match format {
            SampleFormat::F32 => stream::<f32>(&device, &config, queue.clone()),
            SampleFormat::I16 => stream::<i16>(&device, &config, queue.clone()),
            SampleFormat::U16 => stream::<u16>(&device, &config, queue.clone()),
            SampleFormat::I32 => stream::<i32>(&device, &config, queue.clone()),
            other => return Err(format!("the output takes {other} samples, which Zakuro does not make")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Audio {
            queue,
            volume: std::cell::Cell::new(1.0),
            stretch: std::cell::RefCell::new(Stretch::new()),
            capture: std::cell::RefCell::new(Capture::from_env(rate)),
            _stream: stream,
        })
    }

    /// the names of the sound output devices there are.
    pub fn devices() -> Vec<String> {
        let Ok(devices) = cpal::default_host().output_devices() else { return Vec::new() };
        devices.filter_map(|device| device.name().ok()).collect()
    }

    /// how many times the sound ran dry since the last call.
    pub fn take_underruns(&self) -> u64 {
        self.queue.lock().map(|mut queue| std::mem::take(&mut queue.underruns)).unwrap_or(0)
    }

    /// stops the sound while the game is stopped, see Queue::hold.
    pub fn hold(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.hold();
        }
    }

    /// drops the sound queued and half stretched, for a game starting or
    /// stopping, so none of the last one's plays at the start of the next.
    pub fn clear(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.clear();
        }
        self.stretch.borrow_mut().clear();
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.set(volume.clamp(0.0, 1.0));
    }

    /// queues what the console played, stretched when the queue runs low
    /// so that it plays on rather than stops.
    pub fn push(&self, samples: &[[i16; 2]]) {
        let mut capture = self.capture.borrow_mut();
        if capture.as_mut().is_some_and(|capture| capture.feed(samples)) {
            *capture = None;
        }
        let scale = self.volume.get() / 32768.0;
        let samples: Vec<[f32; 2]> = samples.iter().map(|s| s.map(|v| v as f32 * scale)).collect();
        let Ok(mut queue) = self.queue.lock() else { return };
        enqueue(&mut queue, &mut self.stretch.borrow_mut(), &samples);
    }
}

/// adds what the console played to the queue, stretched by how far the
/// queue has run down.
fn enqueue(queue: &mut Queue, stretch: &mut Stretch, samples: &[[f32; 2]]) {
    let below = queue.stretch_below();
    let short = below.saturating_sub(queue.samples.len()) as f64 / below as f64;
    let mut stretched = Vec::with_capacity(samples.len() * 2);
    stretch.process(samples, (1.0 + short).min(MOST_STRETCH), &mut stretched);
    queue.samples.extend(stretched);
    // far ahead, after a stall on the output side, it drops the oldest
    // rather than lag behind the picture
    if queue.samples.len() > queue.target * 4 {
        let excess = queue.samples.len() - queue.target;
        queue.samples.drain(..excess);
    }
}

fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<Mutex<Queue>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let Ok(mut queue) = queue.lock() else { return };
                for frame in out.chunks_mut(channels) {
                    let [left, right] = queue.next();
                    match frame {
                        [mono] => *mono = T::from_sample((left + right) / 2.0),
                        [l, r, rest @ ..] => {
                            *l = T::from_sample(left);
                            *r = T::from_sample(right);
                            rest.fill(T::EQUILIBRIUM);
                        }
                        [] => {}
                    }
                }
            },
            |error| log::warn!("sound output, {error}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(samples: usize, step: f64) -> Queue {
        Queue {
            samples: (0..samples).map(|i| [i as f32, -(i as f32)]).collect(),
            fraction: 0.0,
            step,
            filter: sinc_filter(step),
            starved: false,
            last: [0.0; 2],
            level: 1.0,
            underruns: 0,
            refill: RESUME,
            target: TARGET,
        }
    }

    /// a tone at 440 Hz, the console's rate.
    fn tone(count: usize) -> Vec<[f32; 2]> {
        (0..count)
            .map(|i| {
                let v = (std::f32::consts::TAU * 440.0 * i as f32 / 32728.0).sin() * 0.5;
                [v, v]
            })
            .collect()
    }

    /// how many times a sound crosses zero going up, which tells its pitch.
    fn rises(samples: &[[f32; 2]]) -> usize {
        samples.windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count()
    }

    #[test]
    fn unstretched_sound_comes_out_as_it_went_in() {
        let input = tone(20_000);
        let mut stretch = Stretch::new();
        let mut out = Vec::new();
        for chunk in input.chunks(546) {
            stretch.process(chunk, 1.0, &mut out);
        }
        // past the first piece fading in, every sample is the one that came in
        assert!(out.len() > 18_000);
        for (a, b) in out[HOP..].iter().zip(&input[HOP..]) {
            assert!((a[0] - b[0]).abs() < 1e-4);
        }
    }

    /// a sound that repeats nowhere, with a stretch of silence in the
    /// middle, so where any piece of it came from is clear.
    fn noise_with_a_pause(count: usize) -> Vec<[f32; 2]> {
        let mut x = 1u32;
        (0..count)
            .map(|i| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = if (30_000..33_000).contains(&i) { 0.0 } else { (x >> 8) as f32 / (1 << 24) as f32 - 0.5 };
                [v, v]
            })
            .collect()
    }

    /// a queue a little short of its target is left to DRIFT, its sound goes
    /// in as it came, with no splice.
    #[test]
    fn a_queue_a_little_short_is_not_stretched() {
        let mut queue = queue(TARGET - 1000, 1.0);
        let mut stretch = Stretch::new();
        let input = noise_with_a_pause(20_000);
        let mut out = Vec::new();
        for chunk in input.chunks(546) {
            let before = queue.samples.len();
            enqueue(&mut queue, &mut stretch, chunk);
            out.extend(queue.samples.range(before..).copied());
            // the output plays as much as came in
            queue.samples.drain(..chunk.len());
        }
        assert!(out.len() > 19_000);
        for (a, b) in out[HOP..].iter().zip(&input[HOP..]) {
            assert!((a[0] - b[0]).abs() < 1e-4);
        }
    }

    /// once the queue is full again the stretcher stops, and the sound
    /// follows on as it came in, silence included, with no splice.
    #[test]
    fn stretching_stops_cleanly() {
        let input = noise_with_a_pause(60_000);
        let mut stretch = Stretch::new();
        let mut out = Vec::new();
        for (i, chunk) in input.chunks(546).enumerate() {
            stretch.process(chunk, if i < 20 { 1.5 } else { 1.0 }, &mut out);
        }
        assert_eq!(stretch.factor, 1.0);
        // where the last of it came from, found in noise, which matches in
        // one place only
        let end = &out[out.len() - 10_000..];
        let from = (0..input.len() - end.len())
            .find(|&at| end.iter().zip(&input[at..]).take(64).all(|(a, b)| (a[0] - b[0]).abs() < 1e-4))
            .expect("the end is somewhere in the input");
        // and the input in a row back from there, through the end of the pause
        let tail = &out[out.len() - 28_000..];
        let start = from - 18_000;
        assert!(start < 33_000, "it covers the pause");
        for (a, b) in tail.iter().zip(&input[start..]) {
            assert!((a[0] - b[0]).abs() < 1e-4, "spliced");
        }
    }

    #[test]
    fn stretched_sound_is_longer_at_the_same_pitch() {
        let input = tone(32_728);
        let mut stretch = Stretch::new();
        let mut out = Vec::new();
        for chunk in input.chunks(546) {
            stretch.process(chunk, 1.5, &mut out);
        }
        let ratio = out.len() as f64 / input.len() as f64;
        assert!(ratio > 1.35 && ratio < 1.55, "stretched by {ratio}");
        // the same number of cycles a second
        let pitch = rises(&out) as f64 / out.len() as f64 * 32728.0;
        assert!((pitch - 440.0).abs() < 10.0, "pitch {pitch}");
    }

    /// a straight line of samples comes out on the line, between them.
    #[test]
    fn output_falls_between_the_consoles_samples() {
        let mut queue = queue(TARGET, 0.5);
        let centre = CENTRE as f32;
        for expected in [centre, centre + 0.5, centre + 1.0] {
            let [left, right] = queue.next();
            assert!((left - expected).abs() < 0.01 && (right + expected).abs() < 0.01, "{left} for {expected}");
        }
    }

    /// how strong the frequency is in samples at rate, by the Goertzel
    /// algorithm, as the amplitude of a sine.
    fn strength(samples: &[f32], frequency: f64, rate: f64) -> f64 {
        let coefficient = 2.0 * (std::f64::consts::TAU * frequency / rate).cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for &x in samples {
            let s0 = x as f64 + coefficient * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coefficient * s1 * s2).sqrt() * 2.0 / samples.len() as f64
    }

    /// a sine at the console's rate played out at 48 kHz.
    fn played_at_48k(frequency: f64) -> Vec<f32> {
        const CONSOLE: f64 = 32728.0;
        let mut queue = queue(0, CONSOLE / 48000.0);
        queue.starved = false;
        let sine = |i: usize| (std::f64::consts::TAU * frequency * i as f64 / CONSOLE).sin() as f32 * 0.5;
        let mut made = 0;
        // kept at its target, so DRIFT leaves the pitch alone
        let mut out = Vec::new();
        for _ in 0..8192 {
            while queue.samples.len() < TARGET {
                queue.samples.push_back([sine(made); 2]);
                made += 1;
            }
            out.push(queue.next()[0]);
        }
        // past the filter's start
        out.split_off(TAPS)
    }

    /// a tone well within what the console plays keeps its level.
    #[test]
    fn a_tone_keeps_its_level_at_the_outputs_rate() {
        let level = strength(&played_at_48k(1000.0), 1000.0, 48000.0);
        assert!((level - 0.5).abs() < 0.01, "level {level}");
    }

    /// a high tone does not come out with a false one where its copy above
    /// the console's rate folds down, 32728 - 10000 Hz, as straight
    /// interpolation lets through at about a seventh of the level.
    #[test]
    fn a_high_tone_brings_no_false_tone_with_it() {
        let out = played_at_48k(10_000.0);
        let tone = strength(&out, 10_000.0, 48000.0);
        let false_tone = strength(&out, 22_728.0, 48000.0);
        assert!(tone > 0.45, "the tone itself at {tone}");
        assert!(false_tone < tone / 100.0, "a false tone at {false_tone} beside {tone}");
    }

    /// running dry, the sound fades out slowly rather than stopping short,
    /// and once there is enough again it climbs back rather than jumping.
    #[test]
    fn a_queue_that_ran_dry_fades_out_and_back_in() {
        let mut queue = queue(1, 1.0);
        queue.last = [0.5, 0.5];
        queue.level = 1.0;
        let faded = queue.next();
        assert!(queue.starved);
        assert_eq!(queue.underruns, 1);
        assert!(faded[0] < 0.5 && faded[0] > 0.49, "it fades out rather than stopping short");
        let later = (0..100).map(|_| queue.next()[0]).last().unwrap();
        assert!(later > 0.4, "slowly, at {later} 2 ms on");
        queue.samples.extend((0..RESUME).map(|_| [1.0, 1.0]));
        let first = queue.next()[0];
        assert!(first < 0.95, "it climbs back rather than jumping, at {first}");
        let full = (0..FADE_IN as usize).map(|_| queue.next()[0]).last().unwrap();
        assert!((full - 1.0).abs() < 0.01, "back to full level, at {full}");
    }

    /// stopped while the game is, it does not count running dry, and plays
    /// again only once it is full.
    #[test]
    fn a_held_queue_waits_to_fill_up_and_counts_nothing() {
        let mut queue = queue(0, 1.0);
        queue.last = [0.5, 0.5];
        queue.hold();
        assert!(queue.next()[0] < 0.5);
        assert_eq!(queue.underruns, 0);
        queue.samples.extend((0..RESUME).map(|_| [1.0, 1.0]));
        assert!(queue.next()[0] < 0.5, "what is enough after running dry is not after a hold");
        queue.samples.extend((0..TARGET).map(|_| [1.0, 1.0]));
        let played = (0..FADE_IN as usize).map(|_| queue.next()[0]).last().unwrap();
        assert!((played - 1.0).abs() < 0.01, "it plays once full, at {played}");
        assert_eq!(queue.underruns, 0);
        // running dry later is counted, and waits for the usual amount
        queue.samples.clear();
        queue.next();
        assert_eq!(queue.underruns, 1);
        queue.samples.extend((0..RESUME).map(|_| [1.0, 1.0]));
        assert!(queue.next()[0] > 0.99, "it plays again after the usual amount");
    }

    /// a game starting or stopping drops what the last one queued, and the
    /// new sound plays once there is enough of it.
    #[test]
    fn a_cleared_queue_plays_none_of_the_old_sound() {
        let mut queue = queue(TARGET, 1.0);
        queue.clear();
        queue.next();
        queue.samples.extend((0..TARGET).map(|_| [-1.0, -1.0]));
        // the old sound's left channel was all above zero
        let played: Vec<f32> = (0..FADE_IN as usize).map(|_| queue.next()[0]).collect();
        assert!(played.iter().all(|&v| v <= 0.0), "none of the old sound");
        assert!((played.last().unwrap() + 1.0).abs() < 0.01);
        assert_eq!(queue.underruns, 0);
    }

    /// how far behind the picture the sound is, in ms, after each second of
    /// a minute where the console makes exactly as much sound as the output
    /// plays, but a frame now and then comes late and the next ones catch up,
    /// as when the emulation hitches.
    fn latency_with_hitches() -> Vec<f64> {
        const CONSOLE: f64 = 32728.498;
        const OUTPUT: f64 = 48000.0;
        let mut queue = queue(0, CONSOLE / OUTPUT);
        let mut stretch = Stretch::new();
        let mut seconds = Vec::new();
        let tone = tone(40_000);
        let mut at = 0;
        let mut late: Vec<[f32; 2]> = Vec::new();
        for frame in 0..3600usize {
            // a frame's worth of sound, held back every 45th frame for three
            // frames' time
            let count = (CONSOLE * (frame + 1) as f64 / 60.0) as usize - (CONSOLE * frame as f64 / 60.0) as usize;
            let mut chunk: Vec<[f32; 2]> = (0..count).map(|i| tone[(at + i) % tone.len()]).collect();
            at += count;
            if frame % 45 == 0 {
                late.append(&mut chunk);
            } else if frame % 45 == 3 {
                late.append(&mut chunk);
                enqueue(&mut queue, &mut stretch, &late);
                late.clear();
            } else if late.is_empty() {
                enqueue(&mut queue, &mut stretch, &chunk);
            } else {
                late.append(&mut chunk);
            }
            for _ in 0..(OUTPUT / 60.0) as usize {
                queue.next();
            }
            if frame % 60 == 59 {
                seconds.push(queue.samples.len() as f64 / CONSOLE * 1000.0);
            }
        }
        seconds
    }

    #[test]
    fn hitches_do_not_put_the_sound_further_and_further_behind() {
        let seconds = latency_with_hitches();
        let first = seconds[5];
        let last = *seconds.last().unwrap();
        let most = seconds.iter().cloned().fold(0.0, f64::max);
        eprintln!("latency by second {:?}", seconds.iter().map(|ms| ms.round() as i64).collect::<Vec<_>>());
        assert!(last < first + 30.0, "behind by {first:.0} ms after 5 s and {last:.0} ms after a minute");
        assert!(most < 160.0, "up to {most:.0} ms behind");
    }

    #[test]
    fn a_long_queue_plays_a_little_faster() {
        let mut long = queue(TARGET * 2, 1.0);
        let mut short = queue(TARGET, 1.0);
        for _ in 0..1000 {
            long.next();
            short.next();
        }
        assert!(TARGET * 2 - long.samples.len() > 1000, "the long queue went through more than it played");
        assert!(TARGET - short.samples.len() < 1000, "the short queue went through less than it played");
    }
}
