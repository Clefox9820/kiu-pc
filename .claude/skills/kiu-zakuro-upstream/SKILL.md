---
name: kiu-zakuro-upstream
description: "Trigger: update Zakuro, subtree pull, upstream, fearkov, merge upstream, send changes to Zakuro. Sync the zakuro/ subtree and prepare upstream contributions safely."
license: MIT
metadata:
  author: "kiu-pc contributors"
  version: "1.0"
---

## Activation Contract

Load when pulling newer Zakuro into `zakuro/`, resolving conflicts with upstream, or preparing a change for fearkov/zakuro.

## Hard Rules

- `zakuro/` is a `git subtree --squash` of https://github.com/fearkov/zakuro (MIT); keep `zakuro/LICENSE` and credit.
- Never send AI-written code upstream without human review and disclosure: the author discards fully AI-written contributions.
- Only general fixes go upstream (warm preload, GS mode 2, pipeline keys, diagnostics). KIU-specific parts (mouse aim modes, controls panel, KIU default keymap) stay in this repo.
- Never copy code from GPL projects (for example TriAevum) into this MIT repo without the user deciding to relicense.

## Decision Gates

| Goal | Action |
|---|---|
| Get upstream changes | `git subtree pull --prefix zakuro https://github.com/fearkov/zakuro main --squash` |
| Conflict in our files | Keep our behaviour, re-apply on top of upstream, rerun `kiu-test` |
| Offer a fix upstream | Isolate it in a branch of a fearkov/zakuro fork; the user reviews and submits |

## Execution Steps

1. Ensure a clean worktree; commit or stash first.
2. Run the subtree pull; resolve conflicts preferring upstream except in our features.
3. Rebuild (`kiu-build`, Zakuro-only path) and run all suites (`kiu-test`).
4. Record the upstream commit pulled in the commit message (`chore(zakuro): pull upstream <hash>`).

## Output Contract

Report the upstream commit pulled, conflicts and how they were resolved, test results, and anything left for the user to review.

## References

- `../../../CONTRIBUTING.md` — repository layout and upstream notes.
- `../../../zakuro/LICENSE` — Zakuro's licence.
