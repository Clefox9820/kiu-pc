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

- Close only processes you started, by PID. Never `taskkill /IM kid-icarus.exe`: it closes the user's game too.
- Never run long builds or tests while the user is playing; check `Get-Process kid-icarus` started by them first.
- Inside the Claude desktop app (MSIX), your processes see a virtualized `%APPDATA%` and `%LOCALAPPDATA%`. Changes you make there do not reach the user. To read or write their real files, write a `.bat` with the Write tool on a normal drive and run it with `explorer.exe <bat>`.
- Back up the user's real `settings.toml` before changing it.
- Never answer Windows security prompts (firewall, UAC) for the user.

## Decision Gates

| Need | Do |
|---|---|
| Read the user's log | `.bat` that copies `%APPDATA%\zakuro\zakuro.log` to the repo's ignored scratch area, via `explorer.exe` |
| Change the user's settings | `.bat` that backs up then copies a prepared `settings.toml` |
| Test without disturbing them | your own process, your own PID, outside their play time |

## Execution Steps

1. Check what is running and who started it.
2. Choose the path from the table; write `.bat` files with the Write tool, never shell heredocs (backslashes get mangled).
3. Verify the effect by copying the real file back out and diffing it.

## Output Contract

State which copy (sandbox or real) you read or changed, the PIDs you started and closed, and any prompt left for the user.

## References

- `../kiu-hitch-diagnosis/SKILL.md` — what to do with the real log.
