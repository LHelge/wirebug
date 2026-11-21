---
id: 6af
title: Revisit port re-export aliases now that the BOM counts boundary wires
status: open
priority: P2
created: "2026-11-21T11:25:48.469161270Z"
updated: "2026-11-21T11:25:48.469161270Z"
tags:
  - dsl
  - export
---

## Summary
Found while implementing the BOM export (96q.tt). A component passes a port through its boundary with an internal wire (`wire orange 50 [hv_pos, pack.hv_pos]`, where `hv_pos` is the component's own connectorless port). That wire is a continuation of the conductor the enclosing component declares to the port, not a second piece of wire, but the wire list and the BOM count it on every placement: in `examples/`, 82 of 358 loose conductors (26 of the 61 orange 50 mm²). The BOM flags them in a `boundary` column so a buyer can discount them. `AGENTS.md` (Out of scope, "Port re-export aliases") said to revisit when a BOM makes this hurt.

## Acceptance Criteria
- [ ] Decide: a port alias syntax (`pub port x "X" = inner.port;`, a boundary mapping with no wire), or keep wire-through and have the exports join a boundary continuation with the conductor it continues into one row
- [ ] Either way, the wire list and BOM no longer count one physical conductor twice, and a conductor's length and colour come from one place
- [ ] The related BOM limits get a decision too: a component's internal wiring is listed even when the component is bought as a unit, and a container with ports of its own is listed as an unspecified component (the model has no marker for a purchased sub-assembly)

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.