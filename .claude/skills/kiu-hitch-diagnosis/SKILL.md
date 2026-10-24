---
name: kiu-hitch-diagnosis
description: "Trigger: stutter, hitch, freeze, se pasma, lag spike, shader cache, zakuro.log. Find the cause of frame hitches from Zakuro's per-frame diagnostics."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load when the user reports freezes, stutter or lag, or asks to read `zakuro.log`.

## Hard Rules

- Diagnose from data before changing code; the obvious guess (shader cache) was wrong once.
- Read the user's real log (`%APPDATA%\zakuro\zakuro.log`), not an agent sandbox copy (see `kiu-host-hygiene`).
- Exclude the first ~40 s after launch: the warm preload is still running and startup hitches are expected.
- Check for a concurrent heavy process (gcc builds) before blaming the game; it evicts preloaded pages.

## Decision Gates

Each `hitch:` line has: frame ms, ms since last, instructions interpreted, draws/vertices/textures/GPU wait, recompiled code, services, frame end, sound, GPU syncs, pages brought in.

| Dominant field | Cause | Next step |
|---|---|---|
| pages brought in in the thousands | demand paging from disk | confirm `zakuro::warm` finished; game on HDD? restart game; consider sectional ROM warming |
| recompiled code, few pages, low MIPS in `zakuro_core::slow` | host stall | check page faults and CPU contention |
| instructions interpreted high | code 3dsrecomp missed | re-port to absorb `%APPDATA%\3dsrecomp\<title>.hints` |
| waiting for the GPU / `zakuro_gpu::waits` | GPU-bound | lower resolution, check generic pipeline use |
| `compiled pipeline … on the draw` ≥2 ms | synchronous shader compile | pipeline keys prewarm (`.keys`), dynamic blend support |
| syncs with the GPU | CPU reading GPU-drawn memory | inspect `sync_for_cpu` callers |

## Execution Steps

1. Copy the real log out (via `kiu-host-hygiene`).
2. Summarize hitches per minute: count, total ms, worst, pages.
3. Classify the dominant field with the table; quote 2–3 representative lines.
4. For deeper detail run with `RUST_LOG=info,zakuro_gpu::pipelines=debug` or `zakuro_core::slow`.

## Output Contract

Return the per-minute summary table, the dominant cause with evidence lines, and one recommended next step. Say if the session is invalid (startup only, concurrent build).

## References

- `../kiu-test/SKILL.md` — A/B method for confirming a fix.
