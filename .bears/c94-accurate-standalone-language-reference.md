---
id: c94
title: Accurate, standalone language reference
status: open
priority: P1
created: "2026-11-20T21:40:32.089977524Z"
updated: "2026-11-20T21:49:51.002826892Z"
tags:
  - docs
parent: g2e
---

## Summary
The only DSL spec is the agent skill file, and it is stale: no `enclosure`, no view `text`, `connector_type` missing from the file-contents list, a designator-less `connector "Molex…" {` example that contradicts the required designator, and a loose-wire description ("plain strand group") that no longer matches the renderer.

## Acceptance Criteria
- [ ] Every construct the parser accepts is documented, with one example each
- [ ] Every code block in the reference parses (a test extracts and checks them)
- [ ] The reference lives in a tracked `docs/` directory, split by topic (e.g. components, wires and cables, connectors, views, projects)
- [ ] README links to `docs/` rather than `.agents/skills/`
- [ ] The skill files point at `docs/` for syntax instead of duplicating it, keeping only agent-specific guidance
- [ ] Both skill mirrors (`.claude/skills/`, `.agents/skills/`) and CLAUDE.md / AGENTS.md stay in sync

## Decided (2026-11-20)
Bring back `docs/`. The old wiki dump that sat there has been deleted, so the directory is free.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.