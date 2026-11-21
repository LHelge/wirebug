---
id: kvx
title: "`render` help summary still says \"to SVG\""
status: open
priority: P3
created: "2026-11-21T19:27:28.210447292Z"
updated: "2026-11-21T19:27:28.210447292Z"
tags:
  - cli
---

## Summary
`wirebug render --help` (and `wirebug --help`) summarise the command as "Render every view in a project to SVG", though `--png`, `--pdf` and `--embed` exist. Found in g2e.wj.

## Acceptance Criteria
- [ ] The summary names the output forms
- [ ] Help snapshots updated

## Testing
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.