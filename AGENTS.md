# Agent instructions

This repository builds Kid Icarus Uprising for PC from the user's own ROM:
the game's CPU code is recompiled with 3dsrecomp and linked into the Zakuro
runtime in `zakuro/`. Nothing from the game may ever be committed.

Before working, load the skill that matches the task. Skills live in
`.claude/skills/<name>/SKILL.md`.

| Skill | Load when |
|---|---|
| `kiu-build` | Building, rebuilding, re-translating the game, running setup.ps1 |
| `kiu-test` | Verifying a change, running test suites, measuring performance |
| `kiu-hitch-diagnosis` | Stutter, freezes, reading zakuro.log |
| `kiu-publish-guard` | Any commit, push, PR, release or shared file |
| `kiu-zakuro-upstream` | Updating the zakuro/ subtree or preparing upstream changes |
| `kiu-override-re` | Changing the game's own behaviour (camera, widescreen, UI) |
| `kiu-host-hygiene` | Launching or closing the game, touching the user's settings or logs |

Always:

- Reply to the maintainer in Spanish; write code, comments and docs in English.
- Use Conventional Commits without AI attribution lines.
- Measure before changing performance code, and report what was not verified.
- Keep both releases working: Europe (`build/port`) and USA (`build/port-usa`).
  A change to `overrides/` needs its USA addresses (`scripts/find-usa-addresses.ps1`)
  and a rebuild of both; see `kiu-build`.
