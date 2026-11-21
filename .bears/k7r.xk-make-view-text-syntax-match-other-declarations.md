---
id: k7r.xk
title: Make view `text` syntax match other declarations
status: done
priority: P3
created: "2026-11-20T21:40:32.001396078Z"
updated: "2026-11-21T02:47:38.767548233Z"
tags:
  - dsl
attempts: 1
---

## Summary
`text front_term at (8, 4) "120 Ω termination";` puts the string last, while every other declaration is `<name> "<label>" ...`. It is also undocumented.

## Acceptance Criteria
- [ ] Decide the form (e.g. `text "120 Ω" at (8, 4);`, dropping the unused name) and migrate `examples/`
- [ ] Documented in the language reference

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.

## Decided (2026-11-21, by the orchestrator of the backlog run; the author left the design open)
The form is `text "<label>" at (x, y);`, with no name. The old named form (`text <name> at (x, y) "<label>";`) keeps parsing and rendering, with a `deprecated_text_syntax` warning, because the author's wiring project in another repository still uses it. Removing it is task `xrs`.
