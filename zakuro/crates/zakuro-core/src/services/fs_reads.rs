//! which file of the game's RomFS each of its folders last had read, so the
//! level or scene a game loads shows in the log and in `FsState::last_read`.

use crate::System;

/// the path of an open file that is the game's own RomFS.
pub(super) const ROMFS: &str = "romfs:/";
/// the path of an open file that is the update's RomFS.
pub(super) const PATCH: &str = "patch:/";

/// keeps which file of the game's own RomFS a read was in, by its folder.
// a RomFS with mods over it is read through its layers, which this does not follow.
pub(super) fn note_romfs_read(system: &mut System, path: &str, offset: u64) {
    let Some(title) = system.title.as_ref() else { return };
    let title = match path {
        ROMFS => title,
        PATCH => match title.update() {
            Some(update) => &update.title,
            None => return,
        },
        _ => return,
    };
    let Some(file) = title.romfs.as_ref().and_then(|romfs| romfs.path_at(romfs.base + offset)) else {
        return;
    };
    let (folder, name) = file.rsplit_once('/').unwrap_or(("", file));
    let last_read = &mut system.services.fs.last_read;
    if last_read.get(folder).is_some_and(|last| last == name) {
        return;
    }
    log::debug!("fs: reads {file}");
    last_read.insert(folder.to_owned(), name.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use zakuro_fs::testing::{cia, ncch, write};

    const GAME: u64 = 0x0004_0000_0FF3_DE02;

    /// where a file's data starts in a RomFS, from the RomFS's own start.
    fn offset_in(romfs: &zakuro_fs::romfs::RomFs, path: &str) -> u64 {
        romfs.file_data_offset(&romfs.lookup(path).unwrap()) - romfs.base
    }

    #[test]
    fn a_read_of_the_romfs_is_kept_by_its_folder() {
        let game = write("fs-last-read", "game.cxi", &ncch(GAME, Some(&[0; 8]), &[
            ("us/stage/a0100.zrc", &[1; 64]),
            ("us/stage/b0100.zrc", &[2; 64]),
            ("us/voice/v.bin", &[3; 64]),
        ]));
        let mut system = crate::loader::load(&game, crate::Config::default()).unwrap();
        let at = |system: &System, path: &str| offset_in(system.title.as_ref().unwrap().romfs.as_ref().unwrap(), path);
        for path in ["us/stage/a0100.zrc", "us/voice/v.bin", "us/stage/b0100.zrc"] {
            let offset = at(&system, path);
            note_romfs_read(&mut system, ROMFS, offset + 10);
        }
        let last_read = &system.services.fs.last_read;
        assert_eq!(last_read.get("us/stage").map(String::as_str), Some("b0100.zrc"));
        assert_eq!(last_read.get("us/voice").map(String::as_str), Some("v.bin"));
        // a file that is not the RomFS is not looked into
        let offset = at(&system, "us/stage/a0100.zrc");
        note_romfs_read(&mut system, "sdmc:/save.bin", offset);
        assert_eq!(system.services.fs.last_read.get("us/stage").map(String::as_str), Some("b0100.zrc"));
    }

    /// a read of the update's RomFS is looked up in the update's files, not
    /// the game's, and without an update it is not looked into.
    #[test]
    fn a_read_of_the_updates_romfs_is_kept_from_the_updates_files() {
        let update_id = GAME & 0xFFFF_FFFF | 0x0004_000E_0000_0000;
        let game = write("fs-patch-read", "game.cxi", &ncch(GAME, Some(&[0; 8]), &[("us/stage/a0100.zrc", &[1; 64])]));
        let update = write("fs-patch-read", "update.cia", &cia(update_id, 0x10, &[(0, Some(&ncch(update_id, Some(&[2; 8]), &[
            ("us/stage/a0100.zrc", &[4; 64]),
            ("us/stage/c0200.zrc", &[5; 64]),
        ])))]));

        let mut plain = crate::loader::load(&game, crate::Config::default()).unwrap();
        note_romfs_read(&mut plain, PATCH, 0);
        assert!(plain.services.fs.last_read.is_empty(), "no update, nothing kept");

        let config = crate::Config { update: Some(update), ..crate::Config::default() };
        let mut system = crate::loader::load(&game, config).unwrap();
        let title = system.title.as_ref().unwrap();
        let offset = offset_in(title.update().unwrap().title.romfs.as_ref().unwrap(), "us/stage/c0200.zrc");
        note_romfs_read(&mut system, PATCH, offset + 10);
        assert_eq!(system.services.fs.last_read.get("us/stage").map(String::as_str), Some("c0200.zrc"));
    }
}
