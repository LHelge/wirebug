---
id: g2e.tu
title: LSP hover
status: done
priority: P3
created: "2026-11-20T21:40:32.101788475Z"
updated: "2026-11-21T19:19:32.702978939Z"
tags:
  - lsp
attempts: 1
---

## Summary
Hover on an instance, port, connector or type shows its label, type, pin and where it is defined.

## Implementation Notes
- Reuse the span walk in `src/lsp/definition.rs`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.