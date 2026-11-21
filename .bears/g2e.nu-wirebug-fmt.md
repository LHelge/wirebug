---
id: g2e.nu
title: "`wirebug fmt`"
status: open
priority: P3
created: "2026-11-20T21:40:32.105730769Z"
updated: "2026-11-20T21:40:32.105730769Z"
tags:
  - cli
  - dsl
---

## Summary
A formatter for `.wb` (column-aligned ports and wire endpoint lists, as `examples/` does by hand).

## Implementation Notes
- The lexer already keeps trivia; swap `lex::significant()` for a trivia collector as ARCHITECTURE.md anticipates
- LSP formatting follows from it

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.