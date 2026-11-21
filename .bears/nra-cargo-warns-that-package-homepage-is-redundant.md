---
id: nra
title: Cargo warns that `package.homepage` is redundant
status: open
priority: P3
created: "2026-11-21T19:27:28.217768384Z"
updated: "2026-11-21T19:27:28.217768384Z"
tags:
  - chore
---

## Summary
Every cargo command prints a `redundant_homepage` manifest warning (homepage equals repository). Found during g2e.

## Acceptance Criteria
- [ ] No manifest warning from `cargo build`
- [ ] `cargo publish --dry-run` still builds