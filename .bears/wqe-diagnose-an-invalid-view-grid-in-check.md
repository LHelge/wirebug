---
id: wqe
title: "Diagnose an invalid view `grid:` in `check`"
status: open
priority: P1
created: "2026-11-20T22:44:25.794880702Z"
updated: "2026-11-20T22:44:25.794880702Z"
tags:
  - check
  - render
parent: kss
---

## Summary
Found while implementing 2xa. A schematic `grid:` finer than `MIN_PORT_PITCH / 2` (7.5), or a non-positive `grid:` in any view kind, passes `check --strict` and then fails `render` with `GridTooSmall` / `NonPositiveGrid`. That breaks the epic's rule that a project which passes `check` does not fail in `render` on layout.

## Acceptance Criteria
- [ ] A non-positive `grid:` is an error diagnostic with a span on the value
- [ ] A schematic `grid:` too fine for a port label is an error diagnostic naming the minimum
- [ ] The threshold comes from the renderer's own constant (`Grid::of_view` / `MIN_PORT_PITCH`), so the lint and the renderer cannot disagree
- [ ] A fixture that fails in `render` today fails in `check` instead; the `error::Error` variants stay as the backstop

## Implementation Notes
- The view's `grid` has a span in the AST; 2xa added `Grid::of_view(view) -> Result<Grid>` and the Layout phase (`render::lint::layout_problems`), which today skips a view with an invalid grid — that is the natural place

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.