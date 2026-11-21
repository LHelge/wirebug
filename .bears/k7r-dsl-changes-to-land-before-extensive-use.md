---
id: k7r
title: DSL changes to land before extensive use
type: epic
status: done
priority: P1
created: "2026-11-20T21:40:31.987862998Z"
updated: "2026-11-21T10:01:18.655554686Z"
---

## Scope
Syntax additions and breaks that get more expensive with every `.wb` file written. Each task updates the parser, the docs (the `wirebug-dsl` skill, README), the TextMate grammar, LSP completion, and `examples/`.

## Acceptance Criteria
- [ ] Wires can carry properties
- [ ] Components can carry part data
- [ ] A view can name its subject