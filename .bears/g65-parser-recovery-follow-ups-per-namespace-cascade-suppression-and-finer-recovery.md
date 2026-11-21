---
id: g65
title: "Parser recovery follow-ups: per-namespace cascade suppression and finer recovery"
status: open
priority: P3
created: "2026-11-21T00:58:29.879776515Z"
updated: "2026-11-21T00:58:29.879776515Z"
tags:
  - dsl
  - parse
---

## Summary
Found while implementing hww. Recovery now reports every broken statement, and while any file has a syntax error the problems that report an absence (unknown port, unused import, …) are dropped project-wide so a hole never cascades. That is safe but coarse.

## Acceptance Criteria
- [ ] Suppress absence problems per namespace (only where a hole could have removed the missing thing) rather than project-wide
- [ ] Recover inside `layout face { }` cavities and `twisted { }` without misleading follow-on errors
- [ ] Deleting both the `{` and `}` of one block (braces stay balanced) no longer reports every spilled member
- [ ] The pre-existing `undefined_type` cascades after a `use_not_found` / IO failure get the same treatment

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.