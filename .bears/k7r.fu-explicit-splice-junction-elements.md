---
id: k7r.fu
title: Explicit splice / junction elements
status: done
priority: P3
created: "2026-11-20T21:40:32.019278995Z"
updated: "2026-11-21T04:04:52.402704538Z"
tags:
  - dsl
  - harness
attempts: 1
---

## Summary
Shared rails are loose multi-endpoint wires, so a physical splice has no location, part or appearance in a harness view. Double-crimping two conductors into one cavity stays allowed and covers many cases.

## Acceptance Criteria
- [ ] Design a `splice` member that conductors can end on, and how it draws in a harness

Listed as out-of-scope for the MVP in AGENTS.md — redesign when it lands.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.