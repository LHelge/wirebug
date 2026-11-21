---
id: k7r.z4
title: Explicit view subject (`for <Component>`)
status: open
priority: P2
created: "2026-11-20T21:40:31.995770278Z"
updated: "2026-11-20T21:40:31.995770278Z"
tags:
  - dsl
---

## Summary
A view documents the file's single top-level component, implicitly. A file with two components cannot hold views (`view_subject` error) and the rule is invisible in the source.

## Acceptance Criteria
- [ ] `view schematic "Title" for Sensor { ... }` binds the view to that component
- [ ] Omitting `for` keeps today's behaviour
- [ ] The `view_subject` help text mentions `for`

## Implementation Notes
- `ast::View`, `parse/mod.rs`, `resolve/views.rs` (subject binding), go-to-definition in `lsp/definition.rs`

## Decided (2026-11-20)
The keyword is `for`, placed after the title: `view pinout "Sensor pinout" for Sensor { ... }`.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.