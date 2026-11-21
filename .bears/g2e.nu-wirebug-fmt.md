---
id: g2e.nu
title: "`wirebug fmt`"
status: done
priority: P3
created: "2026-11-20T21:40:32.105730769Z"
updated: "2026-11-21T20:23:17.348557228Z"
tags:
  - cli
  - dsl
attempts: 1
---

## Summary
A formatter for `.wb` (column-aligned ports and wire endpoint lists, as `examples/` does by hand).

## Implementation Notes
- The lexer already keeps trivia; swap `lex::significant()` for a trivia collector as ARCHITECTURE.md anticipates
- LSP formatting follows from it

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.