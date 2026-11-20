---
id: "2td"
title: SVG root carries intrinsic width/height
status: open
priority: P2
created: "2026-11-20T21:40:32.067540501Z"
updated: "2026-11-20T21:40:32.067540501Z"
tags:
  - render
parent: jkx
---

## Summary
The root `<svg>` has only a `viewBox`, so browsers scale every view to the container: a two-box view is blown up and a wide one shrunk, and text size differs between views in the index and in host pages.

## Acceptance Criteria
- [ ] `width`/`height` (world units as px) on the root in standalone mode
- [ ] Index CSS still caps at the container width (`max-width: 100%`)
- [ ] Decide and document the embed-mode behaviour

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.