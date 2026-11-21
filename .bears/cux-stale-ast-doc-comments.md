---
id: cux
title: Stale AST doc comments
status: open
priority: P3
created: "2026-11-21T19:37:59.778601545Z"
updated: "2026-11-21T19:37:59.778601545Z"
tags:
  - docs
---

## Summary
`ast::Port` documents `pins (N, …)` but the grammar is `pins [N, …]`; `ast::Item`'s doc omits `ConnectorType`. Found in g2e.f7.

## Acceptance Criteria
- [ ] AST doc comments match the grammar in `docs/`