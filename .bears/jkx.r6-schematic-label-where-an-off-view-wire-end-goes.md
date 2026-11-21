---
id: jkx.r6
title: "Schematic: label where an off-view wire end goes"
status: done
priority: P2
created: "2026-11-20T21:40:32.053429424Z"
updated: "2026-11-21T15:35:55.476578109Z"
tags:
  - render
  - schematic
attempts: 1
---

## Summary
A listed port whose wire lands on an unlisted or excluded end draws as a bare stub. A small `→ vcm.can2_h` reference would make partial views self-explanatory.

## Acceptance Criteria
- [ ] A listed port with at least one undrawn connection shows a short stub and the far end(s) as `instance.port` (or the instance label)
- [ ] Ports with no wire at all stay bare
- [ ] Many far ends collapse (`→ 3 more`) instead of overflowing

## Edge Cases
- `Own` ends without an enclosure placement
- The label must not collide with routed wires: reserve the stub in the router's obstacle set

## Open Question
Always on, or a per-view switch?

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.