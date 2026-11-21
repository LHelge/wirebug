---
id: jb3
title: An aggregator that merges with `use` lines only gets `unused_import`
status: open
priority: P3
created: "2026-11-21T04:31:53.720155632Z"
updated: "2026-11-21T04:31:53.720155632Z"
tags:
  - dsl
  - check
---

## Summary
Found while implementing k7r.64. An aggregating file that pulls fragments in with `use` only — `use Vehicle from "../main.wb"; use Vehicle from "a.wb";`, named from `main.wb` by `fragment "systems/all.wb";` — merges correctly, but `validate` reports both `use` lines as `unused_import`, so the pattern fails `--strict`. `validate/mod.rs` counts a `use` as used only if the file owns a same-named definition or one of its `fragment` lines merges it. The documented aggregator (a `use` of the component plus `fragment` lines) is clean.

## Acceptance Criteria
- [ ] A `use X` whose collision with another `use X` in the same file performed a merge counts as used
- [ ] A test covers the use-only aggregator under `--strict`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.