---
id: aef
title: "Schematic router: two wires can run on top of each other; titles are not obstacles"
status: open
priority: P2
created: "2026-11-21T12:23:29.816043246Z"
updated: "2026-11-21T12:23:29.816043246Z"
tags:
  - render
  - schematic
---

## Summary
Found while implementing jkx.7u (coloured schematic wires), which makes both more visible.

1. Two different wires can share a run. With boxes a(0,0), b(16,8), c(0,8), d(16,0) and wires a.p–b.p and c.p–d.p (east-facing a.p and c.p, west-facing b.p and d.p), both route along y=120 for x 105..135 and nudging does not separate them, possibly because port-touching segments are pinned. With colour the later wire hides the earlier one there.
2. Component titles are not router obstacles: in the example's "Charging control" view the vertical blue DC SW → DCDC EN wire runs through the "VCM" title.

## Acceptance Criteria
- [ ] A fixture reproduces the shared run; after the fix the two wires are separated (or the layout lints report that they cannot be)
- [ ] Decide whether a component's title is an obstacle, a keep-out the nudger respects, or stays as is
- [ ] `check`'s layout lints and `render` still agree; output stays deterministic

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.