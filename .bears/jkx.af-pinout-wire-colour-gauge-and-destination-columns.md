---
id: jkx.af
title: "Pinout: wire colour, gauge and destination columns"
status: done
priority: P2
created: "2026-11-20T21:40:32.060438068Z"
updated: "2026-11-21T12:32:21.960478816Z"
tags:
  - render
  - pinout
depends_on:
  - 96q.st
  - jkx.r7
attempts: 1
---

## Summary
Turn the pin table into a crimping sheet: for each pin, the conductor(s) in that cavity with colour swatch + code, gauge, and far end.

## Acceptance Criteria
- [ ] Columns come from the per-connector pin table model
- [ ] A cavity holding two conductors (double crimp) shows both
- [ ] The table no longer stretches to the face width when its content is narrow

## Edge Cases
- A pinout view documents a component *type*; conductors belong to a placement. Decide: render against the first instance (like other views) and say so in the header, or omit wire columns when the type is placed more than once.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.