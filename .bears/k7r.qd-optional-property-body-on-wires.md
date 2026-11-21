---
id: k7r.qd
title: Optional property body on wires
status: done
priority: P1
created: "2026-11-20T21:40:31.990543266Z"
updated: "2026-11-21T01:48:33.972158922Z"
tags:
  - dsl
attempts: 1
---

## Summary
Wires are positional only (`wire red 0.5 "L" [a, b];`), leaving no room for per-wire length, insulation type or part number — all needed by the wire list and BOM.

## Syntax (decided 2026-11-20)
```
wire red 0.5 "12V" [a.x, b.y];                 // unchanged
wire red 0.5 "12V" [a.x, b.y] {              // optional body, no trailing `;`
    length: 1.2;
    type:   "FLRY-B";
    part:   "...";
}
```
Same `key: value;` shape as `cable`, same brace rule as `include ... ports { }`.

## Acceptance Criteria
- [ ] Body parses on loose wires and cable conductors (also inside `twisted { }`)
- [ ] `length` (number), `type` (string), `part` (string) land on `ir::Wire`
- [ ] Unknown / duplicate / wrongly-typed keys reuse the cable property diagnostics
- [ ] A conductor `length` inside a cable that also has `length` is defined (override) and documented

## Implementation Notes
- `ast::Wire`, `parse/mod.rs` (boxed), `ir::Wire`, `elaborate`, `validate` (generalise the cable property checks)

## Decided
Trailing optional `{ key: value; }` body, as shown above. The positional form stays; a conductor's `length` overrides its cable's.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.