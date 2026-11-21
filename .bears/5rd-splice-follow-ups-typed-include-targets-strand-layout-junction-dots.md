---
id: 5rd
title: "Splice follow-ups: typed include targets, strand layout, junction dots"
status: open
priority: P3
created: "2026-11-21T04:05:53.713110131Z"
updated: "2026-11-21T04:05:53.713110131Z"
tags:
  - dsl
  - render
---

## Summary
Found while implementing k7r.fu (explicit splices).

## Acceptance Criteria
- [ ] `ir::Include` has a typed target (connector of an instance / own connector for pinouts / splice) in place of reusing `instance` with `connector: None`
- [ ] A splice placed between two conductors of the same cable no longer makes the cable box's strands double back (one cable box per cable on a single spine today)
- [ ] Schematic views draw a junction dot where a rail or a splice net joins, so a crossing wire does not look like a join
- [ ] Decide whether a splice's `description` is drawn in a harness view (only name, label and part are)


## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.