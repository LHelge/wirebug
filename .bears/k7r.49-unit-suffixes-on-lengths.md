---
id: k7r.49
title: Unit suffixes on lengths
status: open
priority: P3
created: "2026-11-20T21:40:31.998603567Z"
updated: "2026-11-20T21:40:31.998603567Z"
tags:
  - dsl
depends_on:
  - k7r.qd
---

## Summary
`length: 2m;` is a parse error (`found 'm' expected ';'`); lengths are bare metres, which reads ambiguously for a 150 mm jumper (`0.15`).

## Acceptance Criteria
- [ ] `length: 2m;`, `150mm;`, `15cm;` parse and normalise to metres in the IR
- [ ] A bare number still means metres
- [ ] An unknown suffix is a diagnostic listing the valid ones

## Implementation Notes
- Lexer number token gains an optional unit; a `Length` newtype in `ir`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.