//! HD versions of a game's small pictures, its HUD's, menus' and portraits',
//! made by an AI upscaler on the player's computer the first time each one
//! shows, and kept there, so that from then on it shows in HD at once.
//!
//! the upscaler is Real-ESRGAN's realesrgan-ncnn-vulkan, a program of its
//! own that runs on the GPU. one picture is made at a time, in tiles and at
//! a low priority, on a thread of its own, so the game goes on meanwhile.
//! nothing is made from anything but the textures the game shows, and the
//! pictures stay in the folder given, on the player's computer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

/// how many times bigger the upscaler makes a picture.
pub const FACTOR: u32 = 4;

/// the model it makes them with, Real-ESRGAN's small one for anime-style
/// art: about 1 s a picture on an integrated GPU, against nearly 3 for its
/// larger realesrgan-x4plus-anime, whose pictures are a little finer.
const MODEL: &str = "realesr-animevideov3";

/// makes the picture at input factor times bigger at output, and says
/// whether it did.
pub(crate) type Upscaler = dyn Fn(&Path, &Path) -> bool + Send + Sync;

/// the folder of HD pictures, which it adds to as they are made.
pub struct HdCache {
    dir: PathBuf,
    /// the pictures in the folder, by the hash of the texture they replace.
    made: Arc<Mutex<HashSet<u64>>>,
    /// asked for this run, made or not, so each is asked for once.
    asked: Mutex<HashSet<u64>>,
    /// made since take_done last looked.
    done: Arc<Mutex<Vec<u64>>>,
    jobs: Mutex<mpsc::Sender<Job>>,
}

struct Job {
    hash: u64,
    texels: Arc<[[u8; 4]]>,
    width: u32,
    height: u32,
}

impl HdCache {
    /// the cache in dir, its pictures made by the realesrgan-ncnn-vulkan at
    /// tool, with its models in the models folder beside it. none when the
    /// program is not there or the folder can't be made.
    pub fn new(tool: &Path, dir: &Path) -> Option<HdCache> {
        if !tool.is_file() {
            log::warn!("no AI upscaler at {}, HD pictures are left out", tool.display());
            return None;
        }
        let models = tool.with_file_name("models");
        let tool = tool.to_path_buf();
        HdCache::with(dir, Box::new(move |input, output| run(&tool, &models, input, output)))
    }

    /// the cache in dir, its pictures made by upscale, for tests that need
    /// no upscaler.
    pub(crate) fn with(dir: &Path, upscale: Box<Upscaler>) -> Option<HdCache> {
        if let Err(error) = std::fs::create_dir_all(dir) {
            log::warn!("could not make {}, {error}, HD pictures are left out", dir.display());
            return None;
        }
        let made: HashSet<u64> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                u64::from_str_radix(name.strip_suffix(".png")?, 16).ok()
            })
            .collect();
        log::info!("{} HD pictures in {}", made.len(), dir.display());
        let made = Arc::new(Mutex::new(made));
        let done = Arc::new(Mutex::new(Vec::new()));
        let (jobs, received) = mpsc::channel::<Job>();
        let (folder, made_by, done_by) = (dir.to_path_buf(), made.clone(), done.clone());
        std::thread::Builder::new()
            .name("hd pictures".to_owned())
            .spawn(move || {
                for job in received {
                    if make(&folder, &job, &*upscale) {
                        made_by.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(job.hash);
                        done_by.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(job.hash);
                    }
                }
            })
            .ok()?;
        Some(HdCache { dir: dir.to_path_buf(), made, asked: Mutex::new(HashSet::new()), done, jobs: Mutex::new(jobs) })
    }

    /// where the HD picture of the texture of hash is, made or not.
    pub fn path(&self, hash: u64) -> PathBuf {
        self.dir.join(format!("{hash:016X}.png"))
    }

    /// whether the HD picture of the texture of hash has been made.
    pub fn made(&self, hash: u64) -> bool {
        self.made.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains(&hash)
    }

    /// has the HD picture of these texels made, once a run however often
    /// it is asked for, after the pictures asked for before it.
    pub fn ask(&self, hash: u64, texels: Arc<[[u8; 4]]>, width: u32, height: u32) {
        if self.made(hash) || !self.asked.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(hash) {
            return;
        }
        let _ = self.jobs.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).send(Job { hash, texels, width, height });
    }

    /// the hashes of the pictures made since it was last called.
    pub fn take_done(&self) -> Vec<u64> {
        std::mem::take(&mut *self.done.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }
}

/// makes one HD picture: the texture written out, made bigger, and put in
/// place under its hash only once whole, so a picture half made never shows.
fn make(dir: &Path, job: &Job, upscale: &Upscaler) -> bool {
    let input = dir.join(format!("{:016X}.in.png", job.hash));
    let output = dir.join(format!("{:016X}.out.png", job.hash));
    let texels: Vec<u8> = job.texels.iter().flatten().copied().collect();
    let made = write_png(&input, job.width, job.height, &texels).is_ok() && upscale(&input, &output);
    let placed = made && std::fs::rename(&output, dir.join(format!("{:016X}.png", job.hash))).is_ok();
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
    if !placed {
        log::warn!("could not make the HD picture of {:016X}", job.hash);
    }
    placed
}

fn write_png(path: &Path, width: u32, height: u32, texels: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut writer| writer.write_image_data(texels)).map_err(|error| error.to_string())
}

/// runs realesrgan-ncnn-vulkan on input, without a window, at a priority
/// below the game's and in tiles of 64, which leave the GPU to the game
/// between them.
fn run(tool: &Path, models: &Path, input: &Path, output: &Path) -> bool {
    let mut command = std::process::Command::new(tool);
    command
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(["-n", MODEL, "-s", &FACTOR.to_string(), "-t", "64", "-m"])
        .arg(models)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    command.status().is_ok_and(|status| status.success()) && output.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zakuro-hd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn wait_done(cache: &HdCache, most: std::time::Duration) -> Vec<u64> {
        let start = std::time::Instant::now();
        loop {
            let done = cache.take_done();
            if !done.is_empty() || start.elapsed() > most {
                return done;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_picture_asked_for_is_made_once_and_found_the_next_run() {
        let dir = folder("once");
        let runs = Arc::new(Mutex::new(0));
        let counted = runs.clone();
        let copy = move |input: &Path, output: &Path| {
            *counted.lock().unwrap() += 1;
            std::fs::copy(input, output).is_ok()
        };
        let cache = HdCache::with(&dir, Box::new(copy)).unwrap();
        let texels: Arc<[[u8; 4]]> = Arc::from(vec![[1, 2, 3, 4]; 4]);
        assert!(!cache.made(0xAB));
        cache.ask(0xAB, texels.clone(), 2, 2);
        cache.ask(0xAB, texels.clone(), 2, 2);
        assert_eq!(wait_done(&cache, std::time::Duration::from_secs(5)), vec![0xAB]);
        assert!(cache.made(0xAB) && cache.path(0xAB).is_file());
        assert_eq!(*runs.lock().unwrap(), 1, "asked twice, made once");
        // nothing half made is left beside it
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

        let again = HdCache::with(&dir, Box::new(|_: &Path, _: &Path| false)).unwrap();
        assert!(again.made(0xAB), "found in the folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_that_fails_is_not_taken_for_made() {
        let dir = folder("fails");
        let cache = HdCache::with(&dir, Box::new(|_: &Path, _: &Path| false)).unwrap();
        cache.ask(0xCD, Arc::from(vec![[0u8; 4]; 4]), 2, 2);
        // the job runs at once, and makes nothing
        assert!(wait_done(&cache, std::time::Duration::from_millis(300)).is_empty());
        assert!(!cache.made(0xCD) && !cache.path(0xCD).exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no files left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_upscaler_no_cache() {
        assert!(HdCache::new(Path::new("Z:/nowhere/realesrgan-ncnn-vulkan.exe"), &folder("none")).is_none());
    }
}
