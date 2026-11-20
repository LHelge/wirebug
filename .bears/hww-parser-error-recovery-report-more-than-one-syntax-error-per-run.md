---
id: hww
title: "Parser error recovery: report more than one syntax error per run"
status: open
priority: P3
created: "2026-11-20T21:40:31.985091667Z"
updated: "2026-11-20T21:40:31.985091667Z"
tags:
  - dsl
  - parse
parent: kss
---

## Summary
A syntax error stops `check` at the first one, so a file with three typos needs three runs, and the LSP shows one squiggle at a time.

## Acceptance Criteria
- [ ] Errors in separate statements/blocks of one file are all reported in one run
- [ ] A file with a broken member still contributes its other definitions to resolve where safe

## Implementation Notes
- chumsky recovery (`recover_with` skipping to `;` / matching `}`) in `src/dsl/parse/mod.rs`; mind the boxing rule in AGENTS.md
- Decide whether a recovered file proceeds to resolve or only reports parse errors

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.