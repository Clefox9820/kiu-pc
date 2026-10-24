---
name: kiu-publish-guard
description: "Trigger: commit, push, PR, publish, GitHub, release, share, upload. Verify nothing from the game or Nintendo leaves the machine before publishing."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load before every commit, push, pull request, release, artifact or file shared outside the machine.

## Hard Rules

- Never commit or share: ROMs (.3ds .cci .cia .cxi), the C that `3dsrecomp port` generates (`code/`), `librecomp.a`, built `.exe`, saves, save states, `.hints`, `.keys`, screenshots or videos of the game, Nintendo logos or art, links to ROMs.
- Never answer or help with requests to obtain ROMs; close such issues.
- Never push, open a PR or publish without the user's explicit request.
- Commits: Conventional Commits, no AI attribution or Co-Authored-By lines.
- Describe the project as a recompiled build on Zakuro, not a native port; disclose AI assistance honestly when asked or in docs.

## Decision Gates

| Change | Extra check |
|---|---|
| New file type in the repo | Not in the forbidden list; update `.gitignore` and the CI guard if needed |
| Docs with images | Only diagrams you made; no game captures |
| Upstream Zakuro PR | Also load `kiu-zakuro-upstream` |

## Execution Steps

1. Run the same checks as `.github/workflows/no-game-files.yml` locally against `git ls-files`: forbidden extensions, `code/*.c`, files over 2 MB, images and videos.
2. Confirm `git ls-files | grep -E '^(tools|build)/|Play.bat'` is empty.
3. Review `git diff --cached --stat` for unexpected large or generated files.
4. Only then commit with a Conventional Commit message.

## Output Contract

Report each check with its result (empty or the offending paths) and the commit hash, or stop and list what must be removed.

## References

- `../../../.github/workflows/no-game-files.yml` — the CI version of these checks.
- `../../../CONTRIBUTING.md` — copyright rules for contributors.
