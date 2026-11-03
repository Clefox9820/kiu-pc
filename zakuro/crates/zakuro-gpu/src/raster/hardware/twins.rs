//! what frame interpolation does for each surface, counted for the log.

use std::collections::HashMap;

use super::Kind;

/// what frame interpolation does for a surface, counted for the log.
#[derive(Clone, Copy)]
pub(super) enum TwinWork {
    /// a draw into its twin.
    Draw,
    /// a display transfer into its twin.
    Transfer,
    /// its twin turned upright for showing.
    Upright,
    /// its image copied into its twin.
    Copy,
}

/// frames of the title's the log of what frame interpolation did covers.
const TWIN_FRAMES: u32 = 600;

/// what frame interpolation did for each surface, by its address, size and
/// kind, and the frames since the log last told.
#[derive(Default)]
pub(super) struct TwinLog {
    work: HashMap<(u32, u32, u32, Kind), [u32; 4]>,
    frames: u32,
}

impl TwinLog {
    /// counts work frame interpolation did for the surface of this shape.
    /// nothing is counted while the log does not tell.
    pub(super) fn count(&mut self, shape: (u32, u32, u32, Kind), work: TwinWork) {
        if !cfg!(test) && !log::log_enabled!(target: "zakuro_gpu::twins", log::Level::Debug) {
            return;
        }
        self.work.entry(shape).or_default()[work as usize] += 1;
    }

    /// a frame of the title's is done. with RUST_LOG=zakuro_gpu::twins=debug,
    /// every TWIN_FRAMES of them, says what frame interpolation did for each
    /// buffer, to tell what the pictures in between cost and where, and
    /// whether it kept every twin, twin_cuts off.
    pub(super) fn frame(&mut self, twin_cuts: bool) {
        self.frames += 1;
        if self.frames < TWIN_FRAMES {
            return;
        }
        self.frames = 0;
        let work = std::mem::take(&mut self.work);
        if !log::log_enabled!(target: "zakuro_gpu::twins", log::Level::Debug) {
            return;
        }
        let mut each: Vec<_> = work.into_iter().collect();
        each.sort_by_key(|&((addr, ..), _)| addr);
        let lines: Vec<String> = each
            .iter()
            .map(|((addr, width, height, kind), [draws, transfers, uprights, copies])| {
                format!("0x{addr:08X} {width}x{height} {kind:?}: {draws} draws, {transfers} transfers, {uprights} uprights, {copies} copies")
            })
            .collect();
        log::debug!(
            target: "zakuro_gpu::twins",
            "pictures in between over {TWIN_FRAMES} frames{}: {}",
            if twin_cuts { "" } else { ", every twin kept" },
            if lines.is_empty() { "none".to_string() } else { lines.join("; ") }
        );
    }

    /// what was counted for the surfaces at an address so far.
    #[cfg(test)]
    pub(super) fn at(&self, addr: u32) -> [u32; 4] {
        let mut sum = [0; 4];
        for (_, work) in self.work.iter().filter(|((at, ..), _)| *at == addr) {
            for (total, count) in sum.iter_mut().zip(work) {
                *total += count;
            }
        }
        sum
    }
}
