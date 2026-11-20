---
id: "2ze"
title: Repository housekeeping
status: open
priority: P2
created: "2026-11-20T21:40:32.097824861Z"
updated: "2026-11-20T21:49:47.370584333Z"
tags:
  - chore
parent: g2e
---

## Summary
`.claude/plans/sunny-foraging-twilight.md` is tracked in the public repo, and an untracked `docs/` held a saved openinverter wiki page from another project (now deleted).

## Acceptance Criteria
- [ ] `git rm --cached .claude/plans/sunny-foraging-twilight.md` and add `.claude/plans/` to `.gitignore` (file stays on disk)
- [x] Delete the openinverter wiki dump from `docs/` (belonged to another project; deleted 2026-11-20, so `docs/` is free for the language reference)
- [x] Commit `.bears/`, `.bears.yml`, `.mcp.json`, `.claude/skills/bears-planning/` and `.claude/agents/planner.md`

## Decided (2026-11-20)
All bears files are committed; the old plan file is untracked and ignored, not deleted.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.