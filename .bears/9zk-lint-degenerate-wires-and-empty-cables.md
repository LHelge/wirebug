---
id: "9zk"
title: Lint degenerate wires and empty cables
status: open
priority: P2
created: "2026-11-20T21:40:31.970832849Z"
updated: "2026-11-20T21:40:31.970832849Z"
tags:
  - check
parent: kss
---

## Summary
A wire that lists the same endpoint twice (`[ecu.gnd, ecu.gnd]`) and a cable with no conductors (`cable c2 { }`) both pass `check --strict`.

## Acceptance Criteria
- [ ] A repeated endpoint within one wire is an error naming both occurrences
- [ ] A cable with zero conductors warns

## Implementation Notes
- `src/dsl/validate/mod.rs`
- Compare resolved endpoints, not text, so `port` vs `inst.port` cannot alias

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.