---
id: g2e.tu
title: LSP hover
status: open
priority: P3
created: "2026-11-20T21:40:32.101788475Z"
updated: "2026-11-20T21:40:32.101788475Z"
tags:
  - lsp
---

## Summary
Hover on an instance, port, connector or type shows its label, type, pin and where it is defined.

## Implementation Notes
- Reuse the span walk in `src/lsp/definition.rs`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.