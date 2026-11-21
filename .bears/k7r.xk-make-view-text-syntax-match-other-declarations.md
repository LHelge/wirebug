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