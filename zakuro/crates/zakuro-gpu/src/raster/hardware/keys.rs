//! the keys of the pipelines a title used, kept in a list beside its
//! pipeline cache, so that the next run makes them all while it starts
//! rather than as it plays.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{PipelineKey, VertexStage, FRAGMENT_CONSTANTS};
use crate::blend::LogicOp;

/// pipelines noted before the list on disk grows by them.
const KEYS_WRITTEN_TOGETHER: usize = 16;
/// a key on disk: the blend register and whether it blends, the logic op
/// as one more than its number or none, depth, the vertex stage, the write
/// mask, then the fragment constants, all little endian.
pub(super) const KEY_BYTES: usize = 12 + 4 * FRAGMENT_CONSTANTS;

/// where the pipelines a title used are listed, beside its cache.
pub(super) fn keys_path(cache: &Path) -> PathBuf {
    cache.with_extension("keys")
}

/// a key as it is kept, none for a translated program's, whose module is
/// made anew each run.
pub(super) fn key_bytes(key: &PipelineKey) -> Option<[u8; KEY_BYTES]> {
    let vertex = match key.vertex {
        VertexStage::Placed => 0,
        VertexStage::Interpreted => 1,
        VertexStage::Translated(_) => return None,
    };
    let mut bytes = [0; KEY_BYTES];
    bytes[..4].copy_from_slice(&key.blend.unwrap_or(0).to_le_bytes());
    bytes[4] = key.blend.is_some() as u8;
    bytes[5] = key.logic_op.and_then(|op| LogicOp::ALL.iter().position(|&each| each == op)).map_or(0, |index| index as u8 + 1);
    bytes[6] = key.depth as u8;
    bytes[7] = vertex;
    bytes[8..12].copy_from_slice(&key.mask.to_le_bytes());
    for (at, constant) in bytes[12..].chunks_exact_mut(4).zip(key.fragment) {
        at.copy_from_slice(&constant.to_le_bytes());
    }
    Some(bytes)
}

fn key_from_bytes(bytes: &[u8]) -> Option<PipelineKey> {
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let vertex = match bytes[7] {
        0 => VertexStage::Placed,
        1 => VertexStage::Interpreted,
        _ => return None,
    };
    let logic_op = match bytes[5] {
        0 => None,
        n => Some(*LogicOp::ALL.get(n as usize - 1)?),
    };
    let mut fragment = [0; FRAGMENT_CONSTANTS];
    for (constant, at) in fragment.iter_mut().zip((12..KEY_BYTES).step_by(4)) {
        *constant = word(at);
    }
    let blend = (bytes[4] != 0).then(|| word(0));
    Some(PipelineKey { blend, logic_op, mask: word(8), depth: bytes[6] != 0, vertex, fragment })
}

/// the pipelines a title used before, from the list beside its cache, each
/// once, none when there is no list or it is unreadable.
fn read_keys(cache: &Path) -> Vec<PipelineKey> {
    let bytes = std::fs::read(keys_path(cache)).unwrap_or_default();
    let mut seen = HashSet::new();
    bytes.chunks_exact(KEY_BYTES).filter_map(key_from_bytes).filter(|key| seen.insert(*key)).collect()
}

/// the keys of the pipelines the title used, those on the list and those
/// noted for it, and those not written there yet.
#[derive(Default)]
pub(super) struct UsedKeys {
    /// the list's file, none for nowhere.
    path: Option<PathBuf>,
    known: HashSet<PipelineKey>,
    new: Vec<PipelineKey>,
}

impl UsedKeys {
    /// the list beside the cache, none when nothing is kept, and the keys
    /// on it, each once, which are not noted again.
    pub(super) fn read(cache: Option<&Path>) -> (UsedKeys, Vec<PipelineKey>) {
        let listed = cache.map(read_keys).unwrap_or_default();
        let used = UsedKeys { path: cache.map(keys_path), known: listed.iter().copied().collect(), new: Vec::new() };
        (used, listed)
    }

    /// notes a pipeline the title used for the list, writing it out now and
    /// then, a few at a time.
    pub(super) fn remember(&mut self, key: PipelineKey) {
        if key_bytes(&key).is_none() || !self.known.insert(key) {
            return;
        }
        self.new.push(key);
        if self.new.len() >= KEYS_WRITTEN_TOGETHER {
            self.write();
        }
    }

    /// adds the pipelines noted since last time to the list on disk.
    pub(super) fn write(&mut self) {
        let Some(path) = &self.path else { return };
        if self.new.is_empty() {
            return;
        }
        let bytes: Vec<u8> = self.new.drain(..).filter_map(|key| key_bytes(&key)).flatten().collect();
        let written = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| {
            use std::io::Write;
            file.write_all(&bytes)
        });
        if let Err(error) = written {
            log::warn!("could not keep the pipelines used in {}, {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipelines_used_are_kept_and_read_back_as_they_were() {
        let mut fragment = [0; FRAGMENT_CONSTANTS];
        fragment[3] = 0xDEAD_BEEF;
        fragment[29] = 7;
        let keys = [
            PipelineKey { blend: Some(0x0123_4567), logic_op: None, mask: 0xF, depth: true, vertex: VertexStage::Placed, fragment },
            PipelineKey { blend: None, logic_op: Some(LogicOp::Xor), mask: 0x7, depth: false, vertex: VertexStage::Interpreted, fragment },
        ];
        let dir = std::env::temp_dir().join(format!("zakuro keys {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = dir.join("title.bin");
        let bytes: Vec<u8> = keys.iter().chain(&keys).filter_map(key_bytes).flatten().collect();
        std::fs::write(keys_path(&cache), bytes).unwrap();
        assert_eq!(read_keys(&cache), keys, "each once, as written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
