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
- Never start `3dsrecomp port` or a full build while the user is playing; it evicts the game's preloaded RAM and causes hitches.
- Close RAM-heavy apps before `3dsrecomp port`: it measures free RAM once at start to size parallel gcc jobs. Closing apps mid-run does not help; restart it instead.
- Keep the GNU toolchain: Rust `x86_64-pc-windows-gnu` with MinGW gcc (`CC=gcc`, `AR=ar`). Do not mix with MSVC.
- Never edit `build/port/code/` by hand; it is generated. Change game behaviour with overrides (`kiu-override-re`).

## Decision Gates

| Situation | Command |
|---|---|
| First build, or 3dsrecomp/hints changed | `scripts\setup.ps1 -Rom <rom>` (about 1 h) |
| ROM unsure | `scripts\setup.ps1 -Rom <rom> -CheckOnly` |
| Force a new translation | add `-Force` |
| Only `zakuro/` changed | `cargo build --release` in `build/port` (2-10 min) |

## Execution Steps

1. Put `tools/cargo/bin` and `tools/mingw64/bin` on PATH; set `RUSTUP_HOME=tools/rustup`, `CARGO_HOME=tools/cargo`, `CC=gcc`, `AR=ar`.
2. Pick the command from the table above. Run builds longer than 2 min in the background.
3. Confirm `build/port/target/release/kid-icarus.exe` was rewritten (timestamp) and the build printed `Finished`.
4. If the exe is locked ("failed to remove file"), the game is running: wait, never kill the user's process.

## Output Contract

Report the command run, its duration, `Finished` or the first error, and the exe path. Say plainly if the build was skipped because the user was playing.

## References

- `../../../scripts/setup.ps1` — the full build pipeline.
- `../../../README.md` — requirements and timings.
