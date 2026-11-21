---
id: ubt
title: "Harness: a conductor between two connectors in the same column is hidden behind the boxes"
status: open
priority: P2
created: "2026-11-21T19:33:27.400381538Z"
updated: "2026-11-21T19:36:34.002551483Z"
tags:
  - render
  - harness
---

## Summary
When both ends of a loose conductor sit in the same grid column of a harness view, the renderer draws a near-vertical bezier between pin dots on the box edges, and the connector boxes cover it: the wire appears to end on a box header, not its pin. Seen in `examples/looms/rear_lighting.wb` (bl_chmsl) and, during g2e.ty review, in three new example views before they were re-laid out.

## Acceptance Criteria
- [ ] A conductor whose ends share a column is routed so both pin ends are visible (e.g. a lane beside the column), or the case is reported by `check` with a layout hint
- [ ] A fixture-based test asserts the drawn path does not cross the interior of either box

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.

## Related, seen in the same work
- Moving connectors so the cable spine shifts can make a cable box overlap a connector table (g2e.ty, first attempt at the interlock view).