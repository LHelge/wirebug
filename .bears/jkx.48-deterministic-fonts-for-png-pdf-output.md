---
id: jkx.48
title: Deterministic fonts for PNG/PDF output
status: done
priority: P3
created: "2026-11-20T21:40:32.074741561Z"
updated: "2026-11-21T15:52:59.393220786Z"
tags:
  - render
attempts: 1
---

## Summary
SVG text is `sans-serif`, so PNG rasterisation depends on whichever font the machine resolves (CI installs one specially), and drawings differ between machines.

## Acceptance Criteria
- [x] Decide: bundle one open-licensed font for resvg/svg2pdf, or document the dependency
- [x] If bundled, PNG output is identical across machines

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.

## Decided
- Bundled: DejaVu Sans 2.37 (regular, bold, oblique; about 2 MB), unmodified from the official release, in `assets/fonts/dejavu/` with its Bitstream Vera / Arev licence and a README (source, checksums), and listed in `Cargo.toml`'s `include`. Bold oblique is left out: no style draws bold italic.
- Compiled in (`include_bytes!`) by `src/render/fonts.rs`, whose font database (those faces only, every generic family mapped to DejaVu Sans, no system fonts) both `PngRasterizer` and `PdfExporter` parse with. Why: the system lookup made PNG/PDF differ between machines and needed a font install in CI, which is now removed.
- SVG output is unchanged (byte-identical, `sans-serif`, no embedded font): it follows the viewer's font, so the text-width estimators stay conservative upper bounds. PDF text stays outlined (`embed_text: false`).