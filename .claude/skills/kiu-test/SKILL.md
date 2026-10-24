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
- On integrated GPUs, run zakuro-gpu with `ZAKURO_GPU_SHADERS=1`; without it 5 raster tests fail (known, not a regression).
- Never judge performance from one run: automated runs varied 4–188 hitches. Use interleaved A/B (a1 b1 a2 b2) with an env switch.
- Never measure during the boot cinematic or the white title screen; measure in real battles.
- Behaviour changes need a test that fails before the fix (RED) and passes after (GREEN) when a deterministic test is possible.

## Decision Gates

| Touched | Run in `zakuro/` |
|---|---|
| `crates/zakuro` | `cargo test -p zakuro --release --lib` |
| `crates/zakuro-core` | `cargo test -p zakuro-core --release --lib` |
| `crates/zakuro-gpu` (incl. Vulkan) | `ZAKURO_GPU_SHADERS=1 cargo test -p zakuro-gpu --release --features vulkan --lib` |
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
