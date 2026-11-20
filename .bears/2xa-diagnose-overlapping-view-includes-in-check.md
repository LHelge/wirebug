---
id: "2xa"
title: Diagnose overlapping view includes in `check`
status: done
priority: P1
created: "2026-11-20T21:40:31.973081959Z"
updated: "2026-11-20T22:43:46.058257237Z"
tags:
  - check
  - render
parent: kss
attempts: 1
---

## Summary
Two harness includes at the same `at (x, y)` silently draw on top of each other, and schematic boxes placed too close make `render` fail with a routing error after `check` said ok.

## Acceptance Criteria
- [ ] Includes whose boxes overlap (schematic, harness, pinout) are reported with both include spans
- [ ] The fixture that reproduces the routing failure now fails in `check`, not `render`

## Implementation Notes
- Box sizes come from the layout code (`render/schematic/layout.rs`, `render/harness/layout.rs`), so this check needs the elaborated design: decide whether it lives in validate (calling layout) or as a pre-render layout pass surfaced through `check_project`
- Keep it a diagnostic with spans, not an `error::Error`

## Edge Cases
- Boxes that touch but do not overlap are fine
- The enclosure is never an obstacle

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.