---
id: b8t
title: Render polish found in the rendered output epic
status: open
priority: P3
created: "2026-11-21T16:06:19.460273380Z"
updated: "2026-11-21T16:06:19.460273380Z"
tags:
  - render
---

## Summary
Small items found while implementing jkx.cp, jkx.48 and jkx.wq.

## Acceptance Criteria
- [ ] `check` warns when a label uses a character the bundled DejaVu Sans has no glyph for (CJK, Thai, Devanagari, …): PNG and PDF draw it as an empty box since no system font is consulted
- [ ] SVG numbers are written with bounded precision: pinout and harness output carries float noise (`-7.799999999999997`, a pinout viewBox `-322.50500000000005 … 645.0100000000001`), which the root `width` and the embed manifest's `width`/`height` now repeat; serde_json without `float_roundtrip` reads such a number back differently
- [ ] The `--png` index shows PNGs at their pixel size (2× world units), so text is twice the size of the SVG index: give the `<img>` an explicit display size
- [ ] Build resvg/fontdb without the system-font features (`fontconfig`, `memmap`): unused since fonts are bundled, smaller binary, and the no-system-font rule is enforced at compile time
- [ ] LSP completion lists nested component names and same-label auto-import items in hash order (cosmetic; clients re-sort)


## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.