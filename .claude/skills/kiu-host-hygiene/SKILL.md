---
name: kiu-host-hygiene
description: "Trigger: run the game, launch, kill process, user's settings, AppData, real log, Claude desktop app, sandbox. Safe rules for running KIU on the user's Windows PC."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.1"
---

## Activation Contract

Load before launching or closing the game, touching Zakuro settings, saves or logs, or running heavy jobs on the user's Windows machine. If `LOCAL.md` sits next to this file, read it too: it holds the rules and paths of this one machine, and it wins where it says more.

## Hard Rules

- Never close a game the user started without their go-ahead, unless `LOCAL.md` grants it. Close only processes you started or were allowed to, by PID (`taskkill /PID <pid>`), never `taskkill /IM`, and say which PID you closed.
- Never overwrite the user's saves or save states; test from a copy of them in a data folder of your own (`--data <folder>`), so their saves and log are never touched.
- Performance measurements (A/B) only while the user is not playing; a second game or a build skews them.
- Test instances run muted: their settings.toml keeps `mute = true` and `volume = 0.0`, checked right before every launch. They also keep `pause_unfocused = false`, or the test game pauses whenever the user clicks another window and its log stops, which looks like a hang.
- Never capture the screen while the user's own game is open: bringing a test window forward steals their focus. Capture only the test instance's window by its PID, and fail rather than fall back to the whole screen, which can show their private windows; delete any such capture at once.
- Inside the Claude desktop app (MSIX), your processes see a virtualized `%APPDATA%` and `%LOCALAPPDATA%`. Changes you make there do not reach the user. To read or write their real files, write a `.bat` with the Write tool on a normal drive and run it with `explorer.exe <bat>`.
- Back up the user's real `settings.toml` before changing it.
- `explorer.exe <bat>` passes no arguments to the `.bat`; have it read them from a small text file instead.
- Write Windows paths into such files with `printf '%s' "$path"` or forward slashes: a path in printf's format string loses `\b`, `\f` and the like to escapes, and the game silently gets a broken path.
- Never answer Windows security prompts (firewall, UAC) for the user.

## Decision Gates

| Need | Do |
|---|---|
| Read the user's log | `.bat` that copies `%APPDATA%\zakuro\zakuro.log` to the repo's ignored scratch area, via `explorer.exe` |
| Change the user's settings | `.bat` that backs up then copies a prepared `settings.toml` |
| Test in game | `.bat` that sets the env vars (probes, `ZAKURO_LOAD_SLOT`, `ZAKURO_SHOT`) and starts a copy of the exe minimized with its own `--data` folder, launched with `explorer.exe`; find its PID by that folder in its command line, close it by PID when done |
| Play or look at the test game unattended | `scripts\drive-window.ps1 -ProcessId <pid> -Steps "click:x,y;key:Return;shot:<png>"`; white shots without the fps overlay mean the display was asleep, which the script wakes before each shot |

## Execution Steps

1. Check what is running and who started it.
2. Choose the path from the table; write `.bat` files with the Write tool, never shell heredocs (backslashes get mangled).
3. Verify the effect by copying the real file back out and diffing it.

## Output Contract

State which copy (sandbox or real) you read or changed, the PIDs you started and closed, and any prompt left for the user.

## References

- `../kiu-hitch-diagnosis/SKILL.md` — what to do with the real log.
- `LOCAL.md` — this machine's own rules and paths, when present (not tracked).
