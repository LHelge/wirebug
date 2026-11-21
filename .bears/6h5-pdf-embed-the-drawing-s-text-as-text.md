---
id: 6h5
title: "PDF: embed the drawing's text as text"
status: open
priority: P3
created: "2026-11-21T16:06:19.436509281Z"
updated: "2026-11-21T16:06:19.436509281Z"
tags:
  - render
  - pdf
---

## Summary
Found while implementing jkx.48. PDF drawing text is converted to outlines (`embed_text: false`). With the bundled DejaVu Sans the example PDF grew from 880 KB to 1.79 MB; embedding the text brings it to about 775 KB and makes wire codes and pin labels searchable and selectable. It was tried and reverted: svg2pdf 0.13.0 keeps a page's fonts in a randomly seeded `HashMap` and writes them in that order (`util/context.rs`, `Context::fonts`), so PDF bytes differ between runs on any page with two or more faces. Rasterised pages are pixel-identical; only object order varies.

## Acceptance Criteria
- [ ] Choose: an upstream svg2pdf fix (an ordered map), a move to its successor krilla / krilla-svg, or accept PDFs whose bytes vary between runs
- [ ] If embedded: `pdftotext` finds a pin label and a wire code; pages look the same; output is byte-deterministic unless knowingly given up


## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.