---
id: r2b
title: "TextMate grammar: automated test, and contextual words in wire endpoints"
status: open
priority: P3
created: "2026-11-21T00:16:15.261038954Z"
updated: "2026-11-21T00:16:15.261038954Z"
tags:
  - vscode
---

## Summary
Found while implementing ftq. There is no automated test of `editors/vscode/syntaxes/wirebug.tmLanguage.json`; ftq checked its new name rules with an ad-hoc simulation. The grammar also still paints a contextual word as a keyword in a bare wire endpoint (`[size]`) and as a cable property value; fixing that needs a bracket-scoped region.

## Acceptance Criteria
- [ ] A `vscode-textmate`-based (or equivalent) test tokenises fixture `.wb` files and asserts scopes for names vs keywords; it runs in CI
- [ ] A contextual word used as a bare wire endpoint or a property value is scoped as a name/value, not a keyword