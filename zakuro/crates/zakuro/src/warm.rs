//! brings the program's own code and the game's ROM into memory on a thread
//! of their own at start. a game linked into the program is hundreds of
//! megabytes of code that the system otherwise reads from disk the first
//! time each part of it runs, in the middle of play, and from a hard disk
//! that is a seek for each, frames the player feels.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// bytes to bring in, and brought in so far, for showing how far it got.
struct Progress {
    total: AtomicU64,
    done: AtomicU64,
}

impl Progress {
    const fn new() -> Progress {
        Progress { total: AtomicU64::new(0), done: AtomicU64::new(0) }
    }

    /// how far along it is, in percent, none when it is not going on.
    fn percent(&self) -> Option<u8> {
        let total = self.total.load(Ordering::Relaxed);
        let done = self.done.load(Ordering::Relaxed);
        (total > 0 && done < total).then(|| (done * 100 / total) as u8)
    }

    fn add(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }

    /// the work is over, however much of it got read: a read that failed or
    /// came up short leaves no progress showing.
    fn finish(&self) {
        self.done.store(self.total.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

static PROGRESS: Progress = Progress::new();

/// how far along it is, in percent, none when it is not going on.
pub fn progress() -> Option<u8> {
    PROGRESS.percent()
}

/// starts warming the program and the ROM at `rom`, when there is one.
pub fn start(rom: Option<&str>) {
    // ZAKURO_NO_WARM=1 leaves it all to the disk, to compare the two
    if crate::dev_env::no_warm() {
        return;
    }
    let rom = rom.map(std::path::PathBuf::from);
    let spawned = std::thread::Builder::new().name("warm".into()).spawn(move || {
        let rom = rom.as_deref().map(|path| (path, std::fs::metadata(path).map_or(0, |m| m.len())));
        bring_in(rom, &PROGRESS);
    });
    if let Err(error) = spawned {
        log::warn!("could not start bringing the code into memory, {error}");
    }
}

/// brings the program's image and the ROM, as big as it is said to be, into
/// memory, counting it in `progress` until it is over.
fn bring_in(rom: Option<(&Path, u64)>, progress: &Progress) {
    let started = std::time::Instant::now();
    progress.total.store(image_size() as u64 + rom.map_or(0, |(_, size)| size), Ordering::Relaxed);
    let code = own_image(progress);
    let read = rom.map_or(0, |(path, _)| read_through(path, progress));
    progress.finish();
    log::info!(
        "brought {} MiB of code and {} MiB of the ROM into memory in {:.1} s",
        code / (1 << 20),
        read / (1 << 20),
        started.elapsed().as_secs_f64()
    );
}

/// reads a file from start to end, which leaves it in the system's cache,
/// where its memory map finds it, and says how much it read.
fn read_through(path: &Path, progress: &Progress) -> usize {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return 0 };
    let mut buffer = vec![0; 8 << 20];
    let mut total = 0;
    while let Ok(read) = file.read(&mut buffer) {
        if read == 0 {
            break;
        }
        total += read;
        progress.add(read as u64);
    }
    total
}

/// where the program's own image starts in memory, and its size, from its
/// headers.
#[cfg(windows)]
fn image() -> Option<(*const u8, usize)> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut u8;
    }
    // SAFETY: a null name asks for the program's own module, which lives as
    // long as the program
    let base = unsafe { GetModuleHandleW(std::ptr::null()) };
    if base.is_null() {
        return None;
    }
    // SAFETY: a loaded module starts with its headers, which give its size
    let size = unsafe {
        let pe = base.add(std::ptr::read_unaligned(base.add(0x3C) as *const u32) as usize);
        // the optional header's SizeOfImage
        std::ptr::read_unaligned(pe.add(0x18 + 0x38) as *const u32) as usize
    };
    Some((base, size))
}

/// touches every page of the program's own image, so the system reads its
/// code in now rather than as each part first runs, and says how much.
#[cfg(windows)]
fn own_image(progress: &Progress) -> usize {
    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut u8,
        allocation_base: *mut u8,
        allocation_protect: u32,
        partition_id: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(address: *const u8, info: *mut MemoryBasicInformation, length: usize) -> usize;
    }
    const MEM_COMMIT: u32 = 0x1000;
    const PAGE_NOACCESS: u32 = 0x01;
    const PAGE_GUARD: u32 = 0x100;
    const PAGE: usize = 4096;

    let Some((base, size)) = image() else { return 0 };
    let mut touched = 0;
    let mut at = base as usize;
    let end = base as usize + size;
    let mut sum = 0u8;
    while at < end {
        // SAFETY: the struct is MEMORY_BASIC_INFORMATION's layout
        let mut info: MemoryBasicInformation = unsafe { std::mem::zeroed() };
        let found = unsafe { VirtualQuery(at as *const u8, &mut info, std::mem::size_of::<MemoryBasicInformation>()) };
        if found == 0 || info.region_size == 0 {
            break;
        }
        let region_end = (info.base_address as usize + info.region_size).min(end);
        let readable = info.state == MEM_COMMIT && info.protect & (PAGE_NOACCESS | PAGE_GUARD) == 0;
        if readable {
            for page in (at..region_end).step_by(PAGE) {
                // SAFETY: a committed, readable page of the program's image
                sum = sum.wrapping_add(unsafe { std::ptr::read_volatile(page as *const u8) });
                touched += PAGE;
                progress.add(PAGE as u64);
            }
        }
        at = region_end;
    }
    std::hint::black_box(sum);
    // what was skipped counts as done, for the progress
    progress.add(size.saturating_sub(touched) as u64);
    touched
}

/// the size of the program's own image, from its headers.
#[cfg(windows)]
fn image_size() -> usize {
    image().map_or(0, |(_, size)| size)
}

#[cfg(not(windows))]
fn image_size() -> usize {
    0
}

/// how many pages the system has brought in for the program so far, the
/// ones read from disk among them.
#[cfg(windows)]
pub fn page_faults() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        page_faults: u32,
        rest: [usize; 8],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut u8;
        fn K32GetProcessMemoryInfo(process: *mut u8, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters { size: std::mem::size_of::<Counters>() as u32, ..Counters::default() };
    // SAFETY: the struct is PROCESS_MEMORY_COUNTERS' layout, its size given
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size) };
    if ok == 0 { 0 } else { counters.page_faults as u64 }
}

#[cfg(not(windows))]
pub fn page_faults() -> u64 {
    0
}

/// Windows only, where the reports came from; mlock or madvise the
/// executable's mapping if other systems hitch the same way.
#[cfg(not(windows))]
fn own_image(_progress: &Progress) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_read_through_to_its_end() {
        let path = std::env::temp_dir().join(format!("zakuro warm {}", std::process::id()));
        std::fs::write(&path, vec![7u8; 3 << 20]).unwrap();
        let progress = Progress::new();
        assert_eq!(read_through(&path, &progress), 3 << 20);
        let _ = std::fs::remove_file(&path);
        #[cfg(windows)]
        assert!(own_image(&progress) > 0, "the test program's own code was touched");
    }

    /// a ROM that reads shorter than its size said, or not at all, still
    /// ends the progress, which would otherwise show over the game for good.
    #[test]
    fn the_progress_ends_with_the_work_however_much_was_read() {
        let path = std::env::temp_dir().join(format!("zakuro warm short {}", std::process::id()));
        std::fs::write(&path, vec![7u8; 1 << 20]).unwrap();
        let progress = Progress::new();
        bring_in(Some((&path, 2 << 20)), &progress);
        let _ = std::fs::remove_file(&path);
        assert_eq!(progress.percent(), None, "a short read");
        let progress = Progress::new();
        bring_in(Some((Path::new("/nowhere/at/all.3ds"), 1 << 20)), &progress);
        assert_eq!(progress.percent(), None, "a ROM that would not open");
    }
}
