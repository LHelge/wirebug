---
id: k7r.fu
title: Explicit splice / junction elements
status: done
priority: P3
created: "2026-11-20T21:40:32.019278995Z"
updated: "2026-11-21T04:04:52.402704538Z"
tags:
  - dsl
  - harness
attempts: 1
---

## Summary
Shared rails are loose multi-endpoint wires, so a physical splice has no location, part or appearance in a harness view. Double-crimping two conductors into one cavity stays allowed and covers many cases.

## Acceptance Criteria
- [ ] Design a `splice` member that conductors can end on, and how it draws in a harness

Listed as out-of-scope for the MVP in AGENTS.md — redesign when it lands.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.

## Decided (2026-11-21, by the orchestrator of the backlog run; the author asked for this to land and left the design open)
`splice <name> ["<label>"];` (optionally with a `{ part: …; description: …; }` body) is a member of a component or `extend` body. A conductor ends on it by its bare name; own ports and splices share one namespace. Each wire to a splice has exactly two ends (`splice_wire_arity`); a shield may not end on one. Existing multi-endpoint rails and double crimps stay valid and unlinted. Harness views place it with `include <splice> at (x, y);` and draw it as a labelled dot its conductors converge on; `undrawn_conductor` judges conductors with a splice end. Schematic views draw the ports a splice joins as one net, with no new syntax. A splice joining fewer than two conductors warns (`splice_joins_nothing`).
