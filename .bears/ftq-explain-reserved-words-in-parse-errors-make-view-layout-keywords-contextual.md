---
id: ftq
title: Explain reserved words in parse errors; make view/layout keywords contextual
status: done
priority: P2
created: "2026-11-20T21:40:31.982579187Z"
updated: "2026-11-21T00:15:24.136889235Z"
tags:
  - dsl
  - parse
parent: kss
attempts: 1
---

## Summary
`pub port size "SIZE"` fails with `found 'size' expected something else`. `size`, `text`, `grid`, `face`, `rows`, `cols`, `at`, `from`, `layout`, `ports` are reserved everywhere, while `type`, `length`, `part`, `male`, `north` are contextual.

## Acceptance Criteria
- [ ] Words only meaningful inside a view / layout / cavity are accepted as port, instance, connector and cable names
- [ ] Using a still-reserved word as a name says `'<word>' is a reserved word` with a help line
- [ ] LSP completion and the TextMate grammar stay correct

## Implementation Notes
- `src/dsl/lex/token.rs` (keyword table), `src/dsl/parse/mod.rs` (an `ident` production that also accepts the contextual tokens; keep it `.boxed()`)
- `src/lsp/complete.rs` scans tokens — check its contexts
- `editors/vscode/syntaxes/wirebug.tmLanguage.json`

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.