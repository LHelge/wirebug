---
id: axt
title: Write errors print their io cause twice
status: open
priority: P3
created: "2026-11-21T19:27:28.199300599Z"
updated: "2026-11-21T19:27:28.199300599Z"
tags:
  - cli
---

## Summary
`error::Error::Write` already puts `{source}` in its message, and `main` prints errors with `{err:#}`, which appends the source again: `error: failed to write afile: File exists (os error 17): File exists (os error 17)`. Found reviewing g2e.xb.

## Acceptance Criteria
- [ ] Each io cause appears once in every CLI error message (pick one place: the variant's message or the chain)
- [ ] A CLI test asserts on one such message

## Testing
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.