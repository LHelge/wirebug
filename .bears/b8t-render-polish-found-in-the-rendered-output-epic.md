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
- [ ] Build resvg/fontdb without the system-font features (`fontconfig`, `memmap`): unused since fonts are bundled, smaller binary, and the no-system-font rule is enforced at compile time
- [ ] LSP completion lists nested component names and same-label auto-import items in hash order (cosmetic; clients re-sort)

- [ ] The identity stamp's revision depends on the user's git environment: `git_revision` (`src/dsl/manifest/git.rs`) appends `-dirty` whenever `git status --porcelain` prints anything, untracked files included, so a file ignored only by one user's GLOBAL excludes makes the same tree `-dirty` elsewhere (CI, a sandbox, another user). Count only tracked changes (`--untracked-files=no` or `git diff --quiet HEAD`); and decide what the stamp shows when git refuses the repository (today the revision silently disappears)
- [ ] Move the schematic's remaining flat text estimates onto the shared `render::text::Font` (jkx.g2): `PIN_CHAR_WIDTH` (pin text extent, the threshold where annotations turn, the `grid_too_fine` minimum), `TITLE_CHAR_WIDTH`, the wire-code and port-label estimates. It changes schematic layout, so compare `check` and renders on every project
- [ ] A view's `.title` (bold 14) is never measured: a long title on a narrow view can run past the viewBox in every renderer
- [ ] A harness cable box on the spine can overlap a connector table when the connectors sit close; nothing reports it, because cable boxes are not includes
- [ ] The class table counts `²`, `–` as a full em though they draw at 0.4–0.5 em, so every `mm²` annotation reserves about 5 px too much; tightening it changes pinout output
- [ ] A stub-only off-view reference keeps its far ends only in an SVG `<title>`; a lightbox of a view in a hidden index tab measures 0×0 (deep links only)

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.