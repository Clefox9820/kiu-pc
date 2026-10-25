---
name: kiu-host-hygiene
description: "Trigger: run the game, launch, kill process, user's settings, AppData, real log, Claude desktop app, sandbox. Safe rules for running KIU on the user's Windows PC."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load before launching or closing the game, touching Zakuro settings, saves or logs, or running heavy jobs on the user's Windows machine.

## Hard Rules

- The maintainer authorized (2026-10-25) the agent to launch, drive and close the game itself for tests, including closing a game they left open when a build or test needs it. Close by PID (`taskkill /PID <pid>`), never `taskkill /IM`, and say which PID you closed.
- Run in-game checks yourself instead of asking the maintainer to play. Their saved states (`%APPDATA%\zakuro\states\<program id>\`) are fixed test points: slot 1 and 2 are combat scenes on foot and in flight. Start one with `ZAKURO_LOAD_SLOT=<n>`; never overwrite those slots.
- The maintainer plays from a copy (`E:\3dsrecomp-work\play\kid-icarus.exe`, refreshed by `Jugar Kid Icarus.bat` when the build is newer) and the agent tests from another (`E:\3dsrecomp-work\test\`), so builds never wait for the game. Build with `build\compilar.ps1`: full speed when nobody plays, idle priority and 6 jobs while the maintainer plays.
- Test runs go through `build\prueba.bat [slot]` (env vars set before `call`): its own exe copy and `--data E:\kiu-pc\build\test-data`, a copy of the maintainer's states and saves, so their saves and log are never touched. Its log is `build\test-data\zakuro.log`. Close only those, by PID: the process whose command line contains `test-data`.
- Performance measurements (A/B) only while the maintainer is not playing; a second game or a build skews them.
- Inside the Claude desktop app (MSIX), your processes see a virtualized `%APPDATA%` and `%LOCALAPPDATA%`. Changes you make there do not reach the user. To read or write their real files, write a `.bat` with the Write tool on a normal drive and run it with `explorer.exe <bat>`.
- Back up the user's real `settings.toml` before changing it.
- Never answer Windows security prompts (firewall, UAC) for the user.

## Decision Gates

| Need | Do |
|---|---|
| Read the user's log | `.bat` that copies `%APPDATA%\zakuro\zakuro.log` to the repo's ignored scratch area, via `explorer.exe` |
| Change the user's settings | `.bat` that backs up then copies a prepared `settings.toml` |
| Test in game | `.bat` that sets the env vars (`ZAKURO_PORTRAITS`, probes) and calls `build\prueba.bat <slot>`, launched with `explorer.exe`; find its PID by the `test-data` command line, close it by PID when done |
| Build the play exe | `powershell -File build\compilar.ps1`, any time; it adapts to whether the game is being played |

## Execution Steps

1. Check what is running and who started it.
2. Choose the path from the table; write `.bat` files with the Write tool, never shell heredocs (backslashes get mangled).
3. Verify the effect by copying the real file back out and diffing it.

## Output Contract

State which copy (sandbox or real) you read or changed, the PIDs you started and closed, and any prompt left for the user.

## References

- `../kiu-hitch-diagnosis/SKILL.md` — what to do with the real log.
