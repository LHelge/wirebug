---
id: pxb
title: Cable shields and drain wires
status: proposed
priority: P3
created: "2026-11-20T21:40:32.012812790Z"
updated: "2026-11-20T21:40:32.015977859Z"
tags:
  - dsl
  - harness
parent: k7r
---

## Summary
The IR has no shield flag, so shielded CAN, resolver and HV cables cannot record or draw their shield and where it terminates.

## Acceptance Criteria
- [ ] Design the syntax (e.g. `shield [a.pin, b.pin];` inside a cable or a `twisted` group) and the harness drawing

Listed as future work in CLAUDE.md — redesign when it lands.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.