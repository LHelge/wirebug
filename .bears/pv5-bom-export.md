---
id: pv5
title: BOM export
status: open
priority: P2
created: "2026-11-20T21:40:32.039298860Z"
updated: "2026-11-20T21:40:32.039298860Z"
tags:
  - export
  - cli
depends_on:
  - "4jm"
  - ekw
  - k69
parent: "96q"
---

## Summary
Aggregate what must be bought: connector housings by `part`, cables by `type` with summed length, loose wire by colour + gauge with summed length, components by part.

## Acceptance Criteria
- [ ] `wirebug export bom` (CSV / JSON)
- [ ] Items lacking a part number are listed, flagged as unspecified, never dropped
- [ ] Quantities are per placement (a type instantiated six times counts six)

## Edge Cases
- An inline contributes both halves
- Conductors without any length are counted but excluded from length totals, and the output says so

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.