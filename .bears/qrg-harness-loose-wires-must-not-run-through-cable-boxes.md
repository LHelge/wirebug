---
id: qrg
title: "Harness: loose wires must not run through cable boxes"
status: open
priority: P2
created: "2026-11-20T21:40:32.063976956Z"
updated: "2026-11-20T21:40:32.063976956Z"
tags:
  - render
  - harness
parent: jkx
---

## Summary
A loose wire is one bezier pin-to-pin and passes straight behind any cable box on the spine, so it reads as a conductor of that cable. Loose wires in the probe also drew with no label / gauge annotation — confirm and fix.

## Acceptance Criteria
- [ ] Loose wires are placed in the spine's occupancy alongside cable boxes (their own slot) so they clear every box
- [ ] Each loose wire carries the `<label> · <gauge>mm² · <code>` annotation

## Implementation Notes
- `src/render/harness/layout.rs` (`build_cable_boxes`, `LooseWire`), `draw.rs::render_loose`
- Not full object avoidance — that stays out of scope

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.