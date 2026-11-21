---
id: jkx.48
title: Deterministic fonts for PNG/PDF output
status: open
priority: P3
created: "2026-11-20T21:40:32.074741561Z"
updated: "2026-11-21T11:47:44.475650181Z"
tags:
  - render
---

## Summary
SVG text is `sans-serif`, so PNG rasterisation depends on whichever font the machine resolves (CI installs one specially), and drawings differ between machines.

## Acceptance Criteria
- [ ] Decide: bundle one open-licensed font for resvg/svg2pdf, or document the dependency
- [ ] If bundled, PNG output is identical across machines

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.