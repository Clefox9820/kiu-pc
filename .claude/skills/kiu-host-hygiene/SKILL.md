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
- The maintainer plays from a copy on the SSD (`C:\kiu\play\kid-icarus.exe`, refreshed by `Jugar Kid Icarus.bat` when the build is newer; from the hard disk E: the game stalled up to a second reading its code back) and the agent tests from another (`E:\3dsrecomp-work\test\`), so builds never wait for the game. Build with `build\compilar.ps1`: full speed when nobody plays, idle priority and 6 jobs while the maintainer plays.
- Test runs go through `build\prueba.bat [slot]` (env vars set before `call`): its own exe copy and `--data E:\kiu-pc\build\test-data`, a copy of the maintainer's states and saves, so their saves and log are never touched. Its log is `build\test-data\zakuro.log`. Close only those, by PID: the process whose command line contains `test-data`.
- Performance measurements (A/B) only while the maintainer is not playing; a second game or a build skews them.
- Test instances run muted: their settings.toml (`build\test-appdata\zakuro\settings.toml` and every variant swapped in) keeps `mute = true` and `volume = 0.0`; a test game at full volume startled the maintainer. They also keep `pause_unfocused = false`, or the test game pauses whenever the maintainer clicks another window and its log stops, which looks like a hang.
- Never capture the screen while the maintainer's own game is open: bringing a test window forward steals their focus. Capture only the test instance's window by its PID, and fail rather than fall back to the whole screen, which can show their private windows; delete any such capture at once.
- Inside the Claude desktop app (MSIX), your processes see a virtualized `%APPDATA%` and `%LOCALAPPDATA%`. Changes you make there do not reach the user. To read or write their real files, write a `.bat` with the Write tool on a normal drive and run it with `explorer.exe <bat>`.
- Back up the user's real `settings.toml` before changing it.
- `explorer.exe <bat>` passes no arguments to the `.bat`; have it read them from a small text file instead (see `build\prueba-fsr.bat`).
- Write Windows paths into such files with `printf '%s' "$path"` or forward slashes: a path in printf's format string loses `\b`, `\f` and the like to escapes, and the game silently gets a broken path.
- Never answer Windows security prompts (firewall, UAC) for the user.
- `E:\3dsrecomp-work\Jugar Kid Icarus.bat` is the maintainer's own launcher (it sets `ZAKURO_PORTRAITS` and `ZAKURO_UPSCALER`); back it up before editing it.
- E: is a slow hard disk: `du` over the whole drive ran for 2+ hours. Measure folder by folder with `timeout`, and read game sizes from Steam `appmanifest_*.acf` (`SizeOnDisk`) and Epic `C:/ProgramData/Epic/EpicGamesLauncher/Data/Manifests/*.item` (`InstallSize`).

## Decision Gates

| Need | Do |
|---|---|
| Read the user's log | `.bat` that copies `%APPDATA%\zakuro\zakuro.log` to the repo's ignored scratch area, via `explorer.exe` |
| Change the user's settings | `.bat` that backs up then copies a prepared `settings.toml` |
| Test in game | `.bat` that sets the env vars (`ZAKURO_PORTRAITS`, probes) and calls `build\prueba.bat <slot>`, launched with `explorer.exe`; find its PID by the `test-data` command line, close it by PID when done |
| Play or look at the test game unattended | `scripts\drive-window.ps1 -ProcessId <pid> -Steps "click:x,y;key:Return;shot:<png>"`; white shots without the fps overlay mean the display was asleep, which the script wakes before each shot |
| Build the play exe | `powershell -File build\compilar.ps1`, any time; it adapts to whether the game is being played |

## Execution Steps

1. Check what is running and who started it.
2. Choose the path from the table; write `.bat` files with the Write tool, never shell heredocs (backslashes get mangled).
3. Verify the effect by copying the real file back out and diffing it.

## Output Contract

State which copy (sandbox or real) you read or changed, the PIDs you started and closed, and any prompt left for the user.

## References

- `../kiu-hitch-diagnosis/SKILL.md` — what to do with the real log.
