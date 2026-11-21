---
id: g2e.xb
title: "`wirebug init` scaffolds a project"
status: done
priority: P2
created: "2026-11-20T21:40:32.093921120Z"
updated: "2026-11-21T19:24:09.984578694Z"
tags:
  - cli
attempts: 1
---

## Summary
Starting a project means hand-writing `wirebug.toml` and `main.wb`.

## Acceptance Criteria
- [ ] `wirebug init [dir]` writes a manifest and a minimal `main.wb` (two components, one wire, one schematic view) that passes `check --strict`
- [ ] Refuses to overwrite an existing manifest

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.