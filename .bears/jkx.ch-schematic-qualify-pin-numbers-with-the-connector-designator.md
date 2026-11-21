---
id: jkx.ch
title: "Schematic: qualify pin numbers with the connector designator"
status: done
priority: P2
created: "2026-11-20T21:40:32.049938707Z"
updated: "2026-11-21T12:40:26.615870694Z"
tags:
  - render
  - schematic
attempts: 1
---

## Summary
A box shows bare pin numbers, so MSD shows pins 1 and 2 twice (HV and interlock connectors) with no way to tell them apart.

## Acceptance Criteria
- [ ] A pin annotation reads `<designator>:<pin>` when the instance has more than one connector
- [ ] Single-connector instances keep the bare number
- [ ] Grid minimum (`MIN_PORT_PITCH`) still holds for the longer text

## Implementation Notes
- `src/render/schematic/layout.rs` / `draw.rs`; `ir::ConnectorRef.name`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.