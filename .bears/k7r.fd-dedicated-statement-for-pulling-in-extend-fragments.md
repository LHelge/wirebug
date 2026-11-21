---
id: k7r.fd
title: Dedicated statement for pulling in `extend` fragments
status: done
priority: P3
created: "2026-11-20T21:40:32.007032838Z"
updated: "2026-11-21T03:06:59.329682670Z"
tags:
  - dsl
attempts: 1
---

## Summary
`main.wb` repeats `use Vehicle from "systems/x.wb";` thirteen times: an import used as a merge trigger. A statement that says what it does (e.g. `fragment "systems/x.wb";`) would read more honestly.

## Acceptance Criteria
- [ ] Decide whether the clarity is worth a second import form; if yes, keep `use` working during a deprecation window

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.

## Decided (2026-11-21, by the orchestrator of the backlog run; the author left the design open)
Yes: `fragment "<path>";` loads a file for its `extend` fragments and views and imports no name. `use X from "<path>";` used as a merge trigger keeps working exactly as before with NO warning and NO deprecation: the author's wiring project relies on it under `--strict`. The "deprecation window" in the acceptance line above was not opened.
