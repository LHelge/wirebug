---
id: jkx.r7
title: "Pinout: sorted rows, viewing-side note, component in header, label fit"
status: done
priority: P1
created: "2026-11-20T21:40:32.056895233Z"
updated: "2026-11-21T12:01:08.035694012Z"
tags:
  - render
  - pinout
attempts: 1
---

## Summary
Four defects in pinout views seen on `Inverter pinouts`: rows follow declaration order (42 after 45; 46, 48, 47, 49); nothing on the drawing says it is viewed from the harness side; the header shows only the designator `control`; `IGNITION` overflows its cavity.

## Acceptance Criteria
- [ ] Table rows sort by pin number
- [ ] Each face carries a `viewed from harness side` caption
- [ ] Header shows the component label plus designator, description and `part`
- [ ] Cavity labels are fitted (shrink or ellipsis) and never cross the cavity border
- [ ] Unused cavities are visually distinct from used ones

## Implementation Notes
- `src/render/pinout/mod.rs`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.