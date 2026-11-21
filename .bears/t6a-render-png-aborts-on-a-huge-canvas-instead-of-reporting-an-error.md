---
id: t6a
title: "`render --png` aborts on a huge canvas instead of reporting an error"
status: open
priority: P1
created: "2026-11-20T23:26:27.410934113Z"
updated: "2026-11-21T01:16:23.244965663Z"
tags:
  - render
  - check
parent: kss
---

## Summary
Found while implementing wqe. Includes placed very far apart (e.g. `include b at (100000, 100000)` with `grid: 20`) pass `check --strict`; `render --png` then aborts the process ("memory allocation of 64015168892928 bytes failed", core dump) rather than failing that view with `PngEncode`. `PngRasterizer::to_png` allocates the pixmap with no size limit. SVG output seemed unaffected.

## Acceptance Criteria
- [ ] Decide a maximum canvas size (pixels) for rasterising
- [ ] A view over the limit fails as a `ViewError` naming the view; the other views are still written
- [ ] Consider a matching Layout-phase diagnostic in `check` so the project does not pass `check` and fail `render`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.