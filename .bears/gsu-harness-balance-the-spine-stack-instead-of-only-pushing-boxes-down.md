---
id: gsu
title: "Harness: balance the spine stack instead of only pushing boxes down"
status: open
priority: P3
created: "2026-11-21T15:37:56.962995370Z"
updated: "2026-11-21T15:37:56.962995370Z"
tags:
  - render
  - harness
---

## Summary
Found while implementing and reviewing jkx.kd. Cable boxes and loose-wire lane slots are stacked along the spine by a sweep that only ever pushes occupants DOWN (`Occupant::stack` in `src/render/harness/layout.rs`). A lane slot inserted between two boxes therefore pushes everything below it: in the fixture's "Pump loom" the Power box moves about 135 px, its leads then cross the lane leads, and the drawing grows.

## Acceptance Criteria
- [ ] The stack is balanced around the occupants' preferred positions (e.g. pool-adjacent-violators) so displacement is shared up and down
- [ ] Views with no loose wires do not change unless the balance genuinely improves them; say which example views move
- [ ] Output stays deterministic; `check`'s canvas lint and the drawing still agree

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.