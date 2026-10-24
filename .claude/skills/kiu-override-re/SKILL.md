---
name: kiu-override-re
description: "Trigger: override, reverse engineering, find function, mouse camera, widescreen, FOV, single screen, mod the game. Change KIU behaviour with 3dsrecomp overrides."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load when a change needs the game itself to behave differently (true mouse camera, widescreen, single-screen UI, gameplay tweaks) rather than Zakuro.

## Hard Rules

- Never state a function's purpose, address or formula without evidence from the binary or a runtime trace; AI-assisted RE has fabricated functions before (acnl-re). Mark unverified findings as such.
- Overrides are hand-written C in this repo; they may reference addresses but must not copy disassembly or game bytes.
- Never edit generated code in `build/port/code/`.
- Every override needs an in-game verification step the user can run, plus a RAM or image check when possible.

## Decision Gates

| Goal | Where to look first |
|---|---|
| Mouse camera on foot | code reading touch input (HID shared memory) and writing camera yaw |
| Widescreen / FOV | code building the projection matrix sent as GPU uniforms |
| Single screen | bottom-screen HUD draw calls and touch handlers |
| Finding an address | trace memory writes/reads in Zakuro around the event, then read that function's ARM |

## Execution Steps

1. Define the observable goal and a check (RAM value, screenshot, behaviour).
2. Locate candidates with a runtime trace (log writes to a watched address, or `zakuro_core::slow` thread/PC hints).
3. Read the function in the generated C or a disassembler; write down evidence (address, inputs, outputs).
4. Write the override (see 3dsrecomp `docs/overrides.md`), pass it with `--overrides`, rebuild (`kiu-build`).
5. Verify with the check from step 1 and ask the user to confirm in play.

## Output Contract

Report the address, the evidence for what it does, the override file, how it was verified, and what remains unverified.

## References

- `../../../tools/3dsrecomp/docs/overrides.md` — override format (present after `scripts/setup.ps1` runs).
- `../kiu-build/SKILL.md` — rebuilding with overrides.
