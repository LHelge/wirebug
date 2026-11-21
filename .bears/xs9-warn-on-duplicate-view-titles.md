---
id: xs9
title: Warn on duplicate view titles
status: done
priority: P3
created: "2026-11-20T21:40:31.975489018Z"
updated: "2026-11-20T22:23:08.607806279Z"
tags:
  - check
parent: kss
attempts: 1
---

## Summary
Two views titled `"S"` render as `s.svg` and `s_2.svg` with no notice; the index then shows two identically named sections.

## Acceptance Criteria
- [ ] A repeated view title (project-wide) warns, labelling both views

## Implementation Notes
- `src/dsl/resolve/views.rs` or validate; keep the slug de-duplication in `render/mod.rs` as the backstop

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.