# kiu-pc

A PC build of the Nintendo 3DS game *Kid Icarus: Uprising* (title ID
`0004000000030200`), made from your own copy of the game.

The game's ARM CPU code is statically recompiled to C with
[3dsrecomp](https://github.com/fearkov/3dsrecomp) (pinned at commit
`c3f21f94ba50d688025febc8974ff348d471b587`) and linked into the runtime of the
[Zakuro](https://github.com/fearkov/zakuro) 3DS emulator, which is included
here in `zakuro/` with changes for this game.

## What is native and what is emulated

Only the game's CPU code is native. In practice about 99.4% of the
instructions the game executes run as recompiled code; the rest falls back to
Zakuro's interpreter.

Everything else is still emulated by Zakuro:

| Part | How it runs |
| --- | --- |
| Game CPU code | Recompiled to C ahead of time, compiled with gcc |
| 3DS OS and services | Emulated (high-level) by Zakuro |
| PICA200 GPU | Translated to Vulkan at run time; on integrated GPUs vertex shading is done on the CPU |
| DSP audio | Simulated on the CPU, played through cpal |
| Cartridge | Read from your ROM file |

All models, textures, music, sound and text are read from your ROM at run
time. This is **not a decompilation**: the generated C is machine-translated
and not meant to be read or edited. Deeper changes to the game's behavior are
done with 3dsrecomp *overrides*, hand-written C functions that replace a game
function by address.

## Legal notice

- You must provide your own **decrypted** dump of the game, made from a
  cartridge or digital copy you own, using your own console.
- Nothing from the game is distributed in this repository: no ROM, no
  translated code, no binaries, no images. The build happens on your machine
  and its output stays in `build/`, which git ignores.
- Do not ask for ROMs or links to them; such requests are closed.
- This project is not affiliated with or endorsed by Nintendo. *Kid Icarus*
  is a trademark of Nintendo.

## Requirements

- Windows 10 or newer, 64-bit.
- About 6 GB of free disk space.
- About one hour for the first build.
- A Vulkan-capable GPU.
- Close memory-heavy programs before building: 3dsrecomp decides how many
  compilers to run in parallel from the free RAM, measured once at the start.

The setup script downloads its own toolchain into `tools/` (WinLibs gcc 16.2
and Rust for the GNU target), so nothing needs to be installed system-wide.

## Building

From the repository root, in PowerShell or a command prompt:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1 -Rom "D:\path\to\your\dump.cci"
```

The script:

1. Checks that the ROM is *Kid Icarus: Uprising* and is decrypted.
2. Installs gcc and Rust into `tools/`.
3. Clones and builds 3dsrecomp at the pinned commit.
4. Translates the game's code into `build/port`.
5. Builds the port against `zakuro/` with optimizations for your CPU.
6. Writes `Play.bat` in the repository root.

Options:

| Switch | Effect |
| --- | --- |
| `-CheckOnly` | Only verify that the ROM is the right game and decrypted, then stop |
| `-Force` | Translate the game again even if `build/port` already exists |

When it finishes, start the game with `Play.bat`.

## Controls

Defaults are laid out for this game. They can be changed in
**Settings > Controls**, where the mouse aim speeds are also set.

| Input | 3DS / action |
| --- | --- |
| W A S D | Move (Circle Pad) |
| Shift | Walk |
| Mouse | Aim (stylus) |
| Left click / Space | Fire (L) |
| Hold right click | On foot (mode 3): put the stylus down and drag, like the touch screen |
| Mouse wheel / Q, E | Pick power (D-pad left/right) |
| Middle click / F | Use power (D-pad down) |
| C | D-pad up |
| R | Reset camera |
| Enter / Backspace | A / B |
| X / T | X / Y |
| P / M | Start / Select |
| N / B | ZL / ZR |
| Numpad | C-Stick |
| 1, 2, 3 | Mouse aim, which also switches by itself: on foot, flying, or off in menus (a key picks one until the game changes mode) |
| 1 | Mouse aim off |
| 2 | Mouse aim for flying: the mouse moves the reticle directly (with the overrides; otherwise the stylus follows the mouse) |
| 3 | Mouse aim on foot: the mouse turns the camera directly (mouse look); hold right click to drag the stylus instead |
| Esc | Emulator menu |
| F11 | Fullscreen |
| F1 | Pause |
| Tab | Fast forward |

A short notice is shown when the aim mode changes. Mode 3 recenters the
stylus instead of letting it reach the screen edge, which avoids the game's
quick "flick" camera spin.

## Performance

The recompiled program is large (the executable is around 424 MiB). When it
and the ROM are loaded on demand from a hard disk, the game stutters every
time it touches code or data it has not used yet.

To avoid that, a warm-up thread runs at start-up: it touches every page of
the program image and reads the ROM from start to end, while the screen shows
"Loading the game into memory, N%". In A/B runs on a hard disk this went from
over 100 hitches and about 13 seconds lost without warm-up to around 10
hitches and under one second with it, none over 150 ms, and none during
gameplay once warm.

- Set `ZAKURO_NO_WARM=1` to disable the warm-up.
- Keeping the ROM and the build on an SSD helps further.
- Vulkan pipelines are cached in `%LOCALAPPDATA%\zakuro\pipelines\`, and the
  list of pipelines the game used is saved next to the cache
  (`<title>.keys`) and compiled at start, so shader compilation stalls happen
  only the first time.

If you still see stutter, the log at `%APPDATA%\zakuro\zakuro.log` contains
`hitch:` lines with a per-frame breakdown; include them in a bug report.

## Status and known issues

- The game runs with the recompiled code; about 99.4% of executed
  instructions are native.
- Mouse aim on foot (mode 3) was recently redesigned and has not yet been
  confirmed in real play. Feedback is welcome.
- On some AMD integrated GPUs, five `zakuro-gpu` raster tests need
  `ZAKURO_GPU_SHADERS=1` to pass. This also happens without this project's
  changes.
- Mouse look on foot, mouse aim while flying and the automatic aim mode
  come from hand-written
  overrides in `overrides/`, found for the European release
  (`0004000000030200`) and not yet confirmed in real play. Builds made
  before it was added need `scripts\setup.ps1 -Force` to get it.
- Widescreen would need more 3dsrecomp overrides and is not done.
- The changes to Zakuro in this repository were written with AI assistance.
  They have not been sent upstream; the Zakuro author does not accept fully
  AI-written contributions, so a human must review them first.

## Credits

- [fearkov](https://github.com/fearkov) for Zakuro and 3dsrecomp, which make
  this possible.
- [WinLibs](https://winlibs.com/) for the gcc build used on Windows.
- The [Rust](https://www.rust-lang.org/) project.

## License

MIT; see [LICENSE](LICENSE). `zakuro/` is Zakuro by fearkov, also MIT
licensed (see `zakuro/LICENSE`), with changes made here.
