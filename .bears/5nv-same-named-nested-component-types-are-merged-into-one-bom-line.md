---
id: 5nv
title: Same-named nested component types are merged into one BOM line
status: open
priority: P3
created: "2026-11-21T11:42:24.919006396Z"
updated: "2026-11-21T11:42:24.919006396Z"
tags:
  - export
  - dsl
---

## Summary
Found in the epic review of 96q. `Front { component Lamp {…} }` and `Rear { component Lamp {…} }`, both without a part number, come out of `wirebug export bom` as one line (`component,,Lamp,…,2,Car.f.l;Car.r.l`), although they are different types; their housings are listed separately. The BOM groups by the bare `TypeName` (`ComponentKey::Type` in `src/export/bom.rs`), and the IR has no type identity beyond that name. View subjects share the limit. README says "each type without one is a row of its own".

## Acceptance Criteria
- [ ] The IR carries a qualified type identity (the path of nested definitions, or the defining file plus name)
- [ ] The BOM groups part-less components by it, and a test covers two nested types of the same name
- [ ] Decide whether view subjects (`for <Component>`) should use it too

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.