---
id: v4h
title: Close the `connector_type` property set like cables, wires and components
status: open
priority: P3
created: "2026-11-21T02:07:36.638887647Z"
updated: "2026-11-21T02:07:36.638887647Z"
tags:
  - dsl
  - check
---

## Summary
Found while implementing k7r.x7. `connector_type` properties are still a free-form `IndexMap<String, ConnectorPropertyValue>` with unchecked keys, while cables, wires and components now have closed key sets checked through `PropertyOwner` (`src/dsl/diagnostics/mod.rs`). A typo in a connector_type key is silently accepted, and its numeric values are never checked.

## Acceptance Criteria
- [ ] A `PropertyOwner::ConnectorType` with the keys the renderers and exports actually read
- [ ] Unknown / duplicate / wrongly-typed keys reuse the shared property diagnostics
- [ ] LSP completion offers the keys from the same table
- [ ] Decide what happens to keys existing projects use that fall outside the set (warning first)

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.