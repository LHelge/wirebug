---
id: fvg
title: Render errors name the view and keep the other views
status: open
priority: P2
created: "2026-11-20T21:40:31.977782301Z"
updated: "2026-11-20T21:40:31.977782301Z"
tags:
  - render
  - cli
depends_on:
  - "2xa"
parent: kss
---

## Summary
`failed to route schematic wire between "sig (SIG)" and "sig (SIG)"` names neither the view nor the instances, and one failing view aborts the whole render with nothing written.

## Acceptance Criteria
- [ ] Every `error::Error` raised while rendering a view carries the view title
- [ ] Routing errors name both endpoints as `instance.port`
- [ ] `render` writes the views that succeeded, reports each failure, and exits non-zero
- [ ] `serve` shows the failure for that view instead of a blank page

## Implementation Notes
- `src/error.rs`, `src/render/mod.rs::render_views`, `src/render/schematic/route/`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.