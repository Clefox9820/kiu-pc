# Changelog

## Unreleased

- Added **Sharp HUD** (off by default): small pictures, the HUD's, menus'
  and portraits', remade twice as big by Anime4K inside the game, with no
  separate program, download or cache.
- Removed the AI option "HD HUD, menus and portraits" (Real-ESRGAN):
  `scripts\setup.ps1` no longer downloads it and `Play.bat` no longer sets
  `ZAKURO_UPSCALER`. A `cache\hd` folder an earlier build left in the data
  folder is no longer read and can be deleted. Settings files that still
  name the option load as before.

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
git clone --branch v0.1.0 https://github.com/Clefox9820/kiu-pc.git
cd kiu-pc
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1 -Rom "D:\path\to\your\dump.cci"
```

### What works

- Both the European and USA releases build with one command; about 99.4% of
  the instructions the game executes run as recompiled code.
- Mouse and keyboard controls laid out for the game, with mouse aim that
  switches by itself between on foot, flying and menus.
- **Esc > Options**: resolution, frame interpolation, screen layout, filter,
  sound, aim speeds and an optional Discord status.
- Drawing on the GPU through Vulkan, vertex shaders included, with a
  pipeline cache and a start-up warm-up that removes most hitches on hard
  disks.
- Single-screen play (work in progress): the "Top screen, the bottom one
  over it in menus" layout, and `ZAKURO_PORTRAITS=1` for dialogue portraits
  and the battle display on the top screen.

### Known issues

- Mouse look on foot, mouse aim while flying and the automatic aim mode come
  from overrides that have not yet been confirmed in long real play; on the
  USA release they were matched by code and not tried in play.
- Local wireless play reaches the warm-up arena, but the battle does not
  start for the player who joined.
- Widescreen (Top screen shape 16:9 or 21:9) was checked in one battle on
  foot and one in flight; cutscenes and menus with 3D were not checked. The
  HUD is still stretched with the screen.
- Performance was measured on one PC only (Ryzen 5 5600G, integrated Radeon,
  Windows 10). Reports from other GPUs are welcome; include the `hitch:` and
  `pacing:` lines from `%APPDATA%\zakuro\zakuro.log`.

The changes to Zakuro in this repository were written with AI assistance and
have not been sent upstream.
