---
id: jkx.g2
title: "Harness: size connector headers from real text widths"
status: done
priority: P1
created: "2026-11-21T16:06:19.413118140Z"
updated: "2026-11-21T16:51:06.942874149Z"
tags:
  - render
  - harness
attempts: 1
---

## Summary
Found in the review of jkx.48 (bundled DejaVu Sans). The harness renderer reserves a flat 7.0 px per character (`src/render/harness/mod.rs`, the `*CHAR_WIDTH` constants) for a connector title set in bold 13 px. DejaVu Sans Bold is wider: "High-mount stop lamp" measures 165 px against 140 + 20 px padding reserved, "OBC/DC-DC" 83 px against 63 + 20. With the font now fixed for PNG and PDF, a long title overruns its box on every machine (it already did wherever the viewer's sans-serif was DejaVu, including CI); see `rear_lighting_loom` in `examples/`.

## Acceptance Criteria
- [ ] The harness sizes a connector's header (title, part/description line) and its table columns from per-character widths of the bundled font, the way the pinout does (`src/render/pinout/fit.rs`, `fit::Font`): one shared estimator in `render/`, not a second copy
- [ ] No title, subtitle or table cell overruns its box in PNG/PDF for the fixtures and `examples/`; a test measures it
- [ ] The estimate stays an upper bound for viewers whose sans-serif is narrower (SVG output follows the viewer's font)
- [ ] `check`'s overlap and canvas lints still agree with the drawing; output stays deterministic
- [ ] Say which existing layouts change size (wider boxes can newly overlap: `overlapping_includes`)


## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.