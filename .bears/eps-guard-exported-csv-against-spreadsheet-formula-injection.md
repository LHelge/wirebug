---
id: eps
title: Guard exported CSV against spreadsheet formula injection
status: open
priority: P3
created: "2026-11-21T10:57:55.061618948Z"
updated: "2026-11-21T10:57:55.061618948Z"
tags:
  - export
  - cli
---

## Summary
Found while implementing 96q.4v. `wirebug export … --format csv` writes labels verbatim (correctly quoted per RFC 4180), so a field beginning with `=`, `+`, `-` or `@` is evaluated as a formula when the file is opened in a spreadsheet. Labels come from the project's own `.wb` files, so the risk is low, but a shared library file could carry one.

## Acceptance Criteria
- [ ] Decide the guard (prefix such fields with a `'`, or an opt-in flag) without corrupting legitimate values such as a negative number in a numeric column
- [ ] JSON output is unaffected
- [ ] A test with a label starting with `=`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.