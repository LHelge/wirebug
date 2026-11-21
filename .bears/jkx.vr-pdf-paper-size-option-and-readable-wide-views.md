---
id: jkx.vr
title: "PDF: paper size option and readable wide views"
status: done
priority: P2
created: "2026-11-20T21:40:32.071141627Z"
updated: "2026-11-21T13:03:11.480102366Z"
tags:
  - render
  - pdf
  - cli
attempts: 1
---

## Summary
PDF is A4 only; `HV System Overview` fitted to A4 landscape leaves wire codes and pin numbers around 3 pt.

## Acceptance Criteria
- [ ] `--paper a4|a3|letter|tabloid` (default a4)
- [ ] A view whose text would fall below a legibility threshold on the chosen paper produces a warning naming the view

## Implementation Notes
- `src/render/pdf.rs`, `src/cli.rs`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.