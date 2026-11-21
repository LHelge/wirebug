---
id: g2e.wj
title: "`render --out` defaults to `out/`"
status: open
priority: P1
created: "2026-11-20T21:40:32.086188982Z"
updated: "2026-11-20T21:40:32.086188982Z"
tags:
  - cli
---

## Summary
The README shows `wirebug render` with no arguments, but `--out` is required and the command exits 2.

## Acceptance Criteria
- [ ] `--out` defaults to `<project root>/out`
- [ ] README and `--help` agree
- [ ] CLI test for the no-argument form

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.