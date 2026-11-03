# Contributing

Thank you for helping. Please read the first section before anything else.

## The copyright rule

This repository holds only code written by people. Never commit, attach or
link to:

- ROMs, dumps or any file extracted from the game;
- links to places to download the game;
- the C code that `3dsrecomp port` generates (it is the game's code,
  translated);
- compiled output (`.exe`, `.a`, `.o`, and similar);
- saves, save states, `.hints` or `.keys` files;
- screenshots or videos of the game, or Nintendo logos and art.

Diagrams you made yourself are welcome, as SVG: the workflow below blocks
every raster image, since it cannot tell a diagram from a screenshot.

The `no game files` workflow in `.github/workflows/` reads every commit a
push or pull request adds, not only the last one, and blocks dump, binary,
game audio and icon extensions, generated `code/*.c`, anything under
`tools/` or `build/`, `Play.bat`, files over 2 MB, and images or videos.
`.github/scripts/no-game-files.sh` runs the same check locally, for example
`git rev-list origin/main..HEAD | bash .github/scripts/no-game-files.sh`.
Issues and pull requests asking for ROMs or game files are closed without
discussion.

## Repository layout

| Path | Contents |
| --- | --- |
| `zakuro/` | Zakuro, imported with `git subtree --squash` from fearkov/zakuro at `70b56de`, plus this project's changes |
| `overrides/` | Hand-written C that replaces game functions by their European address, and `usa-addresses.psd1`, the same addresses in the USA release |
| `scripts/setup.ps1` | Windows setup: checks the ROM, installs tools, builds 3dsrecomp, translates and builds the game |
| `scripts/env.ps1` | The toolchain environment setup.ps1 uses, to dot-source in your own shell |
| `scripts/lib/` | PowerShell modules the scripts share: ROM header reading, the USA address table |
| `scripts/find-usa-addresses.ps1` | For maintainers: finds the USA address of each override address from both ROMs and checks the table |
| `scripts/drive-window.ps1` | For maintainers: drives and captures a test game window for unattended checks |
| `.github/` | CI guard (`workflows/`, `scripts/`) and issue templates |
| `.claude/`, `AGENTS.md` | Instructions and skills for AI coding agents working on this repository |
| `tools/`, `build/`, `Play.bat` | Created by the setup script; ignored by git |

## Updating Zakuro from upstream

```sh
git subtree pull --prefix zakuro https://github.com/fearkov/zakuro main --squash
```

Resolve conflicts with our changes, then run the tests below. Do not
reintroduce upstream's game screenshots.

## Building and testing

Run `scripts/setup.ps1` once to get the toolchain in `tools/`. To use it from
PowerShell, dot-source the same settings setup.ps1 uses, from the repository
root:

```powershell
. scripts\env.ps1
```

From Git Bash, the same settings are:

```sh
export RUSTUP_HOME="$PWD/tools/rustup"
export CARGO_HOME="$PWD/tools/cargo"
export PATH="$CARGO_HOME/bin:$PWD/tools/mingw64/bin:$PATH"
export CC=gcc
export AR=ar
```

Then:

```sh
cd zakuro
cargo test -p zakuro --release --lib
cargo test -p zakuro-core --release --lib
cargo test -p zakuro-gpu --release --features vulkan --lib
```

The game shades vertices on a dedicated GPU and on the CPU with an integrated
one; `ZAKURO_CPU_SHADERS=1` or `ZAKURO_GPU_SHADERS=1` picks one. The tests shade
on the GPU; with `ZAKURO_CPU_SHADERS=1` five raster tests fail on some AMD
integrated GPUs.

## Reading the diagnostics

The log is at `%APPDATA%\zakuro\zakuro.log`.

- `hitch:` lines mark slow frames and give a per-frame breakdown of where
  the time went.
- `zakuro_core::slow` lines name recompiled calls that took 20 ms or more.
- Page fault counts show whether the program or ROM was still being loaded
  from disk (see the warm-up in the README; `ZAKURO_NO_WARM=1` turns it off
  for comparison).

Linked ports also write interpreter hints to
`%APPDATA%\3dsrecomp\<title>.hints`. These are derived from the game and must
not be committed.

## Where help is most wanted

- **Land aim feedback.** Mouse aim mode 3 (on foot) is new and has not been
  confirmed in real play. Reports on how it feels, and when it misbehaves,
  are very useful.
- **Overrides.** Mouse look on foot, mouse aim while flying, the automatic
  aim mode and the widescreen camera come from 3dsrecomp overrides:
  hand-written C replacing specific game functions by address. They have
  not been confirmed in long real play, and the USA release's were matched
  by code but not tried in play; reports and fixes are welcome. Overrides
  are human-written and may be committed; the generated code around them
  may not. A new override address needs its USA one in
  `overrides/usa-addresses.psd1`, or the USA build stops.
- **Upstreaming.** Several changes in `zakuro/` are general fixes (for
  example, geometry shader mode 2 and the pipeline key list) that could
  benefit Zakuro itself.

### Note on AI-written changes

The changes to Zakuro in this repository were written with AI assistance. The
Zakuro author does not accept fully AI-written contributions. Anything sent
upstream must first be reviewed, understood and taken responsibility for by
a human, and follow upstream's own contribution rules.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/), for example
`fix(zakuro): ...`, `feat(setup): ...`, `docs: ...`. Keep each commit to one
coherent change with its tests and docs.
