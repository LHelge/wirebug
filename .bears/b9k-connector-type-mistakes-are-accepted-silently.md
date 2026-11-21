---
id: b9k
title: "`connector_type` mistakes are accepted silently"
status: open
priority: P2
created: "2026-11-21T19:37:59.767217107Z"
updated: "2026-11-21T19:37:59.767217107Z"
tags:
  - dsl
  - check
---

## Summary
Found while writing the `docs/` reference (g2e.f7):
- `layout grid { numbering: <mode>; }` with an unknown mode silently falls back to `row_major` (`render/pinout/mod.rs`, `GridNumbering::from`).
- `cavity … size <x>` with anything but `large` silently means size 1.
- A `connector_type` with two `layout` blocks keeps the last one (`parse/mod.rs`, `layout = Some(next_layout)`).
- Property keys other than `part` are accepted unchecked and unused (the docs call them "kept as notes").

## Acceptance Criteria
- [ ] Unknown numbering modes and cavity sizes are diagnostics naming the accepted values
- [ ] A duplicate `layout` is a diagnostic pointing at both
- [ ] Unknown keys are either a warning or documented as free-form notes, decided explicitly
- [ ] `docs/connectors.md` and the skill match

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.