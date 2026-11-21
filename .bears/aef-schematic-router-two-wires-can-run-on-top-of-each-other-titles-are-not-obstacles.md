---
id: aef
title: "Schematic router: two wires can run on top of each other; titles are not obstacles"
status: open
priority: P1
created: "2026-11-21T12:23:29.816043246Z"
updated: "2026-11-21T15:37:56.936005349Z"
tags:
  - render
  - schematic
---

## Summary
Found while implementing jkx.7u (coloured schematic wires), which makes both more visible.

1. Two different wires can share a run. With boxes a(0,0), b(16,8), c(0,8), d(16,0) and wires a.p–b.p and c.p–d.p (east-facing a.p and c.p, west-facing b.p and d.p), both route along y=120 for x 105..135 and nudging does not separate them, possibly because port-touching segments are pinned. With colour the later wire hides the earlier one there.
2. Component titles are not router obstacles: in the example's "Charging control" view the vertical blue DC SW → DCDC EN wire runs through the "VCM" title.
3. Pin annotations and wires collide (found while implementing jkx.ch, which lengthens annotations to `<designator>:<pin>`): the router does not see annotations, so a channel wire can run straight through one (under the batteries in the example's HV overview, the orange wire crosses `bms_signals:6`), and a level west/east annotation can be crossed by its own wire's first bend. The `wire-code` placement does not treat annotations as obstacles either, and north-side annotations run into the centred component title.
4. Found while implementing jkx.r6 (off-view references): `wire-code` texts are placed without regard to off-view reference texts either, and can sit on them at the minimum grid; and which way the router breaks an equal-cost tie is fragile when a route starts with a bend (in the example's cooling view a reroute round the fan could go east or west, and west crosses FAN 12V).

## Acceptance Criteria
- [ ] A fixture reproduces the shared run; after the fix the two wires are separated (or the layout lints report that they cannot be)
- [ ] Decide whether a component's title is an obstacle, a keep-out the nudger respects, or stays as is
- [ ] Pin annotations stay readable: a clear lane for the text, the side of the stub away from the wire's own bend, or annotations as obstacles for the router and for `wire-code` placement (making them obstacles for `wire-code` alone moved codes in single-connector views, so decide it together)
- [ ] `check`'s layout lints and `render` still agree; output stays deterministic

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.