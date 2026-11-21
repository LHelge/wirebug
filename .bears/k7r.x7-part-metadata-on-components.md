---
id: k7r.x7
title: Part metadata on components
status: done
priority: P2
created: "2026-11-20T21:40:31.993150435Z"
updated: "2026-11-21T02:06:51.176305089Z"
tags:
  - dsl
attempts: 1
---

## Summary
Only `connector_type` has `part:`. Components (contactor, fuse, sensor) have nowhere to record a part number, manufacturer or rating, so a BOM cannot list them.

## Acceptance Criteria
- [ ] `component X { part: "..."; manufacturer: "..."; ... }` properties parse and reach `ir::Instance`
- [ ] Unknown keys are diagnosed like cable properties
- [ ] `extend` fragments cannot redeclare a property set on the root

## Implementation Notes
- Settle the key set with the BOM task; keep it closed (no free-form map) per the type-system rules in AGENTS.md

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.