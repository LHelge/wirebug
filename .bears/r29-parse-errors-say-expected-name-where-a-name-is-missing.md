---
id: r29
title: Parse errors say `expected name` where a name is missing
status: open
priority: P3
created: "2026-11-21T00:16:15.255961165Z"
updated: "2026-11-21T00:16:15.255961165Z"
tags:
  - dsl
  - parse
---

## Summary
Found while implementing ftq. The parser's single `ident` production is unlabelled, so chumsky reports `expected something else` where a name is missing. `.labelled("name")` would give `expected name`. It changes every such message, so it is its own change.

## Acceptance Criteria
- [ ] A missing name reads `expected name` in parse errors
- [ ] Tests that assert parse messages are updated deliberately

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.