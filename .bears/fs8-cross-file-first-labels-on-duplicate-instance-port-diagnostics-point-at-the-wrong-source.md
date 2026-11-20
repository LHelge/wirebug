---
id: fs8
title: Cross-file `first` labels on duplicate instance/port diagnostics point at the wrong source
status: open
priority: P2
created: "2026-11-20T22:23:37.148540325Z"
updated: "2026-11-20T22:23:37.148540325Z"
tags:
  - check
  - lsp
---

## Summary
Found while implementing xs9. In `src/dsl/resolve/merge.rs`, `DuplicateInstance` and `DuplicatePort` raised across `extend` fragments use the repeat's file as `src`, but their `first` span can come from another fragment's file. miette would then render that label against the wrong text and the LSP would place it wrongly. Not yet reproduced.

## Acceptance Criteria
- [ ] A fixture/unit test with the same instance (and port) name declared in two fragment files reproduces or disproves the wrong label
- [ ] If real: the first occurrence is carried as a miette `#[related]` note with its own source, the pattern `Problem::DuplicateViewTitle` / `FirstViewTitle` introduced (the LSP already converts related notes)

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.