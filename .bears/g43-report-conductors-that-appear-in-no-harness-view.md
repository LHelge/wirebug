---
id: g43
title: Report conductors that appear in no harness view
status: done
priority: P1
created: "2026-11-20T21:40:31.980192898Z"
updated: "2026-11-20T23:07:01.119934029Z"
tags:
  - check
  - harness
parent: kss
attempts: 1
---

## Summary
Harness views silently drop a wire unless both ends land on included connectors, so a conductor can be missing from every build drawing unnoticed.

## Acceptance Criteria
- [ ] For a component that has at least one harness view, each wire segment whose two ends are both on connectors but which no harness view draws is reported (warning)
- [ ] The message names the wire/cable and both endpoints
- [ ] Components with no harness view are not reported (they are not being built from drawings yet)

## Implementation Notes
- Reuse the harness layout's kept-conductor decision (`render/harness/layout.rs`) rather than re-deriving it, so the lint and the drawing cannot disagree
- Needs `ir::Design`, so it runs after elaboration

## Edge Cases
- Multi-endpoint rails: judge each chain segment
- Inline connectors: a conductor counts as covered when the view includes either half

## Decided (2026-11-20)
A default warning (fatal under `--strict`), scoped as in the acceptance criteria: only components with at least one harness view, only wires with both ends on connectors. No `--coverage` flag.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.