---
id: a5p
title: Dedicated statement for pulling in `extend` fragments
status: proposed
priority: P3
created: "2026-11-20T21:40:32.007032838Z"
updated: "2026-11-20T21:40:32.009919338Z"
tags:
  - dsl
parent: k7r
---

## Summary
`main.wb` repeats `use Vehicle from "systems/x.wb";` thirteen times: an import used as a merge trigger. A statement that says what it does (e.g. `fragment "systems/x.wb";`) would read more honestly.

## Acceptance Criteria
- [ ] Decide whether the clarity is worth a second import form; if yes, keep `use` working during a deprecation window

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.