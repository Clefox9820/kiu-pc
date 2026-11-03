---
name: kiu-build
description: "Trigger: build, rebuild, compile, setup, port, recompile Kid Icarus, cargo build. Build the KIU PC executable from the user's own ROM with 3dsrecomp + Zakuro."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load when building the game for the first time, rebuilding after a change in `zakuro/`, or re-translating the game's code.

## Hard Rules

- Never commit or share anything under `build/` or `tools/`; it is game code or machine-specific. Load `kiu-publish-guard` before any commit.
- Never start `3dsrecomp port` while the user is playing; it evicts the game's preloaded RAM and causes hitches. A `cargo build` is fine through `build\compilar.ps1`, which drops to idle priority while they play.
- On the maintainer's PC the game they play is built from `E:\3dsrecomp-work\kid-icarus`, which links `E:\3dsrecomp-work\zakuro-latest`, not this repo's `zakuro/` (`build/port` holds only `code/`). Copy each changed `zakuro/` file there with CRLF line endings (`sed 's/\r$//; s/$/\r/'`), back up the original first, then run `build\compilar.ps1`.
- Close RAM-heavy apps before `3dsrecomp port`: it measures free RAM once at start to size parallel gcc jobs. Closing apps mid-run does not help; restart it instead.
- Keep the GNU toolchain: Rust `x86_64-pc-windows-gnu` with MinGW gcc (`CC=gcc`, `AR=ar`). Do not mix with MSVC.
- `sed -i` in Git Bash rewrites a CRLF source file as LF; edit with the Edit tool, or run `unix2dos` on the file afterwards.
- Never edit `build/port/code/` by hand; it is generated. Change game behaviour with overrides (`kiu-override-re`).
- Two releases are supported: Europe (`0004000000030200`, built in `build/port`) and USA (`0004000000030100`, built in `build/port-usa`). Every change must reach both: rebuild each port the change affects, never only the one in use. `Play.bat` points at whichever was built last.
- `3dsrecomp port` always recompiles every file (about 75 min). To add or change an override without that, regenerate the C into a scratch folder with no gcc on PATH (it stops right after `wrote N files`), diff it against the build's `code\`, compile only the changed `.c`, `entries.c` and `hand\*.c` with `-O2 -ffp-contract=off -fno-math-errno -w --param ggc-min-expand=20 --param ggc-min-heapsize=65536 -I .`, and `ar r` them into a backed-up `librecomp.a`. This only links when the regeneration used the same interpreter hints as the archive: the EUR play build's are in `E:\3dsrecomp-work\kid-icarus\play-build.hints` (put it at `%APPDATA%\3dsrecomp\0004000000030200.hints`); `build\port-usa` matches empty hints. If `entries.o` reports undefined `f_` symbols, the hints differ: do a full port.
- After editing `zakuro-gpu/shaders/{raster.frag,raster.vert,shade.vert}`, rebuild their `.spv` with `tools\glslang\bin\glslang.exe -V --target-env vulkan1.0` (`-DWRITES_DEPTH` for `raster_depth.frag.spv`); naga cannot compile `raster.frag`'s combined samplers. Run the zakuro-gpu suite after: its GPU tests compare against software.
- Other sessions edit `E:\3dsrecomp-work\zakuro-latest` too: apply your own edits there (CRLF) instead of copying whole files over it, and diff first.
- Before rebuilding the USA port after an override change, run `scripts\find-usa-addresses.ps1 -Europe <eur rom> -Usa <usa rom>`; it must print `overrides\usa-addresses.psd1 has every USA address`. A USA dump whose header says encrypted while its content is not is rejected by setup.ps1 and 3dsrecomp (find-usa-addresses.ps1 still reads it); use a dump with the NoCrypto flag set.
- Windows PowerShell 5.1 reads a BOM-less `.ps1` as ANSI: in a test script, build non-ASCII characters with `[char]0xE9` and the like instead of typing them, or the test mangles its own paths.

## Decision Gates

| Situation | Command |
|---|---|
| First build | `scripts\setup.ps1 -Rom <rom>` (about 75 min) |
| ROM unsure | `scripts\setup.ps1 -Rom <rom> -CheckOnly` |
| Only `zakuro/` changed | `cargo build --release` in `build/port` and in `build/port-usa` (2-10 min each); on the maintainer's PC, copy to `zakuro-latest` and run `build\compilar.ps1` (3-4 min at full speed) |
| An override changed or was added, or 3dsrecomp/hints changed | `find-usa-addresses.ps1` first, then `scripts\setup.ps1 -Rom <rom> -Force` once with each ROM (about 75 min each) |

## Execution Steps

1. Put `tools/cargo/bin` and `tools/mingw64/bin` on PATH; set `RUSTUP_HOME=tools/rustup`, `CARGO_HOME=tools/cargo`, `CC=gcc`, `AR=ar`.
2. Pick the command from the table above. Run builds longer than 2 min in the background.
3. Confirm `target/release/kid-icarus.exe` in each rebuilt port was rewritten (timestamp) and the build printed `Finished`.
4. If the exe is locked ("failed to remove file"), the game is running: wait, never kill the user's process.

## Output Contract

Report the command run, its duration, `Finished` or the first error, and the exe path, for each release. Say plainly which release was not rebuilt, and why (no ROM for it, user playing).

## References

- `../../../scripts/setup.ps1` — the full build pipeline.
- `../../../README.md` — requirements and timings.
