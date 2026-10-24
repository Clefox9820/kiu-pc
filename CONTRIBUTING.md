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

The `no game files` workflow in `.github/workflows/` blocks dump and binary
extensions, generated `code/*.c`, files over 2 MB, and images or videos.
Issues and pull requests asking for ROMs or game files are closed without
discussion.

## Repository layout

| Path | Contents |
| --- | --- |
| `zakuro/` | Zakuro, imported with `git subtree --squash` from fearkov/zakuro at `70b56de`, plus this project's changes |
| `scripts/setup.ps1` | Windows setup: checks the ROM, installs tools, builds 3dsrecomp, translates and builds the game |
| `.github/` | CI guard and issue templates |
| `tools/`, `build/`, `Play.bat` | Created by the setup script; ignored by git |

## Updating Zakuro from upstream

```sh
git subtree pull --prefix zakuro https://github.com/fearkov/zakuro main --squash
```

Resolve conflicts with our changes, then run the tests below. Do not
reintroduce upstream's game screenshots.

## Building and testing

Run `scripts/setup.ps1` once to get the toolchain in `tools/`. To use it from
a shell (Git Bash shown):

```sh
export RUSTUP_HOME="$PWD/tools/rustup"
export CARGO_HOME="$PWD/tools/cargo"
export PATH="$PWD/tools/mingw64/bin:$CARGO_HOME/bin:$PATH"
export CC=gcc
```

Then:

```sh
cd zakuro
cargo test -p zakuro --release --lib
cargo test -p zakuro-core --release --lib
ZAKURO_GPU_SHADERS=1 cargo test -p zakuro-gpu --release --features vulkan --lib
```

`ZAKURO_GPU_SHADERS=1` is needed on some AMD integrated GPUs, where five
raster tests fail without it.

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
- **Overrides.** A true mouse-controlled camera and widescreen would need
  3dsrecomp overrides: hand-written C replacing specific game functions by
  address. Overrides are human-written and may be committed; the generated
  code around them may not.
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
