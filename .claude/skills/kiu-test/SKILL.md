---
name: kiu-test
description: "Trigger: test, run tests, verify, benchmark, measure performance, A/B, regression. Run Zakuro test suites and measure the KIU build reliably."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load before claiming a change works, after any edit under `zakuro/`, and for any performance claim.

## Hard Rules

- Run the suites for every crate you touched; report failures, never hide them.
- The game shades vertices on the CPU with an integrated GPU (the maintainer's 5600G) and on a dedicated GPU; `ZAKURO_GPU_SHADERS=1` or `ZAKURO_CPU_SHADERS=1` forces one for an A/B. The tests shade on the GPU; with `ZAKURO_CPU_SHADERS=1` 5 raster tests fail on integrated GPUs (known, not a regression).
- `ZAKURO_GPU_TIMES=1` logs the GPU's time per kind of work every 600 batches (`zakuro_gpu::times`); it stays valid with the display off, unlike pacing. Check whether a cost scales with resolution before optimizing pixels: on the 5600G it did not, the vertex shading was the cost.
- Never judge performance from one run: automated runs varied 4–188 hitches. Use interleaved A/B (a1 b1 a2 b2) with an env switch.
- Never measure during the boot cinematic or the white title screen; measure in real battles.
- Measure frame pacing (unshown frames, present times) only with the display on: Windows turns it off after 15 idle minutes on this PC and then throttles presentation to ~33 fps, whatever the build or settings. CPU and GPU times and screen dumps (`ZAKURO_SHOT`) stay valid with it off.
- Compare builds by screen dumps: runs from the same save state with no input draw the same frames, so `ZAKURO_SHOT` PNGs of two builds must match pixel for pixel; a mouse or keyboard touching the test window breaks that.
- Test copies use `build\test-appdata`, which the maintainer may also play and change (they raised its volume once): check `mute = true` and `volume = 0.0` right before every launch, and put root keys before the first `[table]` of `settings.toml`, or they land inside it.
- Start test copies minimized (`start "" /min ...` in a `.bat`) and capture with `ZAKURO_SHOT`, not the window, so the maintainer's screen is left alone.
- `pacing:` counts are meaningless while the maintainer plays or builds; compare per-frame work with `RUST_LOG=info,zakuro::frames=debug` (took, drawing, waiting) and interleaved runs.
- `ZAKURO_SHOT` saves the screens before the presenter, so it never shows a screen filter (FSR, Sharp); check those with `scripts\drive-window.ps1` `shot:` on a window bigger than the resolution (for example `resolution = 1`, `scale = 2`).
- Compare sound between builds with `ZAKURO_WAV=<file.wav>` (`ZAKURO_WAV_FROM`, `ZAKURO_WAV_SECONDS`) from the same save state with no input; `build\port-usa` builds straight from this repo's `zakuro/`, so it is the port to switch between code versions, then rebuild it on the branch.
- To see whether a texture replacement is used at all, make it fully transparent in a throwaway build: if the picture still shows, the draw never takes it.
- Behaviour changes need a test that fails before the fix (RED) and passes after (GREEN) when a deterministic test is possible.
- A raster test that rewrites a texture's memory between draws must put it at a new address: the texture cache keeps what it decoded, so the second draw silently samples the old bytes.
- The GPU raster tests draw with `raster_depth.frag.spv`; a shader mutation check has to rebuild both `.spv` variants or it proves nothing.

## Decision Gates

| Touched | Run in `zakuro/` |
|---|---|
| `crates/zakuro` | `cargo test -p zakuro --release --lib` |
| `crates/zakuro-core` | `cargo test -p zakuro-core --release --lib` |
| `crates/zakuro-gpu` (incl. Vulkan) | `cargo test -p zakuro-gpu --release --features vulkan --lib` |
| Performance | headless benchmark, then A/B in real play |

## Execution Steps

1. Set up the toolchain env as in `kiu-build`.
2. Run the suites from the table; record `test result` lines.
3. Quick benchmark: `kid-icarus.exe <rom> --headless 1200` and record `Completed after N frames in S`.
4. For stutter claims: run the same scripted session twice with and twice without the change (for example `ZAKURO_NO_WARM=1`) and compare hitch count, total ms and hitches ≥150 ms from the log (see `kiu-hitch-diagnosis`).
5. Prefer deterministic input: `ZAKURO_RECORD=<file>` to record, `ZAKURO_REPLAY=<file>` to replay.

## Output Contract

Return a table: suite or scenario, command, result (passed/failed counts or hitch totals). State what was not covered (for example "not tested in a land battle").

## References

- `../../../CONTRIBUTING.md` — testing section.
- `../kiu-hitch-diagnosis/SKILL.md` — reading hitch lines.
