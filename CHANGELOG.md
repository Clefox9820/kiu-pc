# Changelog

## v0.1.0 — first release

The first public release of kiu-pc: a PC build of *Kid Icarus: Uprising*
made from your own decrypted dump of the European (`0004000000030200`) or
USA (`0004000000030100`) release. The game's CPU code is recompiled with
3dsrecomp and linked into the Zakuro 3DS emulator runtime, which emulates
everything else. It is not a native port or a decompilation.

**This release is source only.** No game files, translated code or
executables are included or ever will be: `scripts\setup.ps1` builds the
game on your machine from your ROM. See the README for requirements (Windows
10+, Vulkan 1.3, about 6 GB of disk and 75 minutes per release).

To get later updates with `git pull`, clone the repository instead of
downloading the source archive:

```powershell
git clone https://github.com/Clefox9820/kiu-pc.git
cd kiu-pc
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1 -Rom "D:\path\to\your\dump.cci"
```

### What works

- Both the European and USA releases build with one command; about 99.4% of
  the instructions the game executes run as recompiled code. Setup checks
  the gcc and Rust it downloads and pins the Rust version.
- Mouse and keyboard controls laid out for the game, with mouse aim that
  switches by itself between on foot, flying and menus.
- **Esc > Options** opens Zakuro's settings in the game's own look:
  resolution, frame interpolation, screen layout, filter and sound; the
  mouse aim speeds, inverting the aim and taking the mouse back under
  **Controls**; pausing in the background, the Discord status and Fullscreen
  under **Interface**.
- Drawing on the GPU through Vulkan, vertex shaders included, with a
  pipeline cache and a start-up warm-up that removes most hitches on hard
  disks.
- Widescreen (Top screen shape 16:9 or 21:9) widens the 3D scene and keeps
  the top screen's HUD in proportion: each 2D piece along an edge is
  squeezed back toward that edge and the rest, such as markers, around its
  own middle. Portraits and badge figures stay in place.
- **Sharp HUD** (off by default): small pictures, the HUD's, menus' and
  portraits', remade twice as big by Anime4K inside the game, with no
  separate program, download or cache.
- The **Anime4K** screen filter: like FSR, but each screen is made twice as
  big by Anime4K's network on the GPU, for sharper edges and text.
  **Anime4K + Restore** cleans lines and letters first, at twice the GPU
  time.
- Single-screen play (work in progress): the "Top screen, the bottom one
  over it in menus" layout, and `ZAKURO_PORTRAITS=1` for dialogue portraits
  and the battle display on the top screen.

### Known issues

- Mouse look on foot, mouse aim while flying and the automatic aim mode come
  from overrides that have not yet been confirmed in long real play; on the
  USA release they were matched by code and not tried in play.
- Local wireless play reaches the warm-up arena, but the battle does not
  start for the player who joined.
- Widescreen was checked in one battle on foot and one in flight; cutscenes
  and menus with 3D were not checked.
- Performance was measured on one PC only (Ryzen 5 5600G, integrated Radeon,
  Windows 10). Reports from other GPUs are welcome; include the `hitch:` and
  `pacing:` lines from `%APPDATA%\zakuro\zakuro.log`.

### If you built from an earlier clone

Run `scripts\setup.ps1` again after `git pull`. Since that clone:

- The AI option "HD HUD, menus and portraits" (Real-ESRGAN) is gone: setup
  no longer downloads it and `Play.bat` no longer sets `ZAKURO_UPSCALER`. A
  `cache\hd` folder in the data folder is no longer read and can be deleted.
  Settings files that still name the option load as before, and every
  setting keeps its name in `settings.toml`.
- A saved state from another Zakuro version loads only when that version is
  known to save states the same way (0.2.29 for this 0.2.31); any other is
  turned down, naming its version.
- Setup no longer stops with "struct `Host` has no field named `svc`", and
  writes `Cargo.toml`, `build.rs` and `Play.bat` as UTF-8, so paths with
  accents work; `Play.bat` finds the program next to itself and takes a ROM
  path with `%`.
- Fixed: the controller's Home button left the mouse aim on over the pause
  menu; turning off "Take the mouse back" had no effect after switching
  windows; the "Loading the game into memory" notice could stay up for good;
  once the game had made more than 128 cameras, widescreen could keep
  widening a camera's view.
- `svcGetHandleInfo` answers only for the game's own process, and returns
  the console's errors for anything else.
- For contributors: the USA build stops when an override names an address
  with no USA address yet (the table is `overrides\usa-addresses.psd1`), and
  the `no game files` check reads every commit a push adds.

The changes to Zakuro in this repository were written with AI assistance and
have not been sent upstream.
