---
id: kss.ag
title: Lint non-positive wire gauge and cable length
status: done
priority: P1
created: "2026-11-20T21:40:31.968700101Z"
updated: "2026-11-20T22:13:35.528815544Z"
tags:
  - check
attempts: 1
---

## Summary
`wire black -1 [...]`, gauge `0`, and `length: -2;` all pass `check --strict` today (the cable box even renders `-2 m`).

## Acceptance Criteria
- [ ] Gauge <= 0 is an error with a span on the number
- [ ] Cable `length` <= 0 is an error

## Implementation Notes
- `src/dsl/validate/mod.rs` (next to the cable property checks, so a type instantiated many times reports once)
- New `Problem` variants in `src/dsl/diagnostics/mod.rs`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.