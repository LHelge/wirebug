---
id: kss
title: "Harden `check`: catch real mistakes before render"
type: epic
status: open
priority: P1
created: "2026-11-20T21:40:31.964595699Z"
updated: "2026-11-20T21:40:31.964595699Z"
---

## Scope
Close the gaps where `check --strict` passes a project that is wrong or that `render` then rejects.

## Acceptance Criteria
- [ ] Nonsense values (gauge, length) are diagnosed
- [ ] A project that passes `check` does not fail in `render` on layout
- [ ] A conductor missing from every harness view is reported

## Decided
Two conductors in one cavity is NOT a problem — it is a deliberate double crimp that saves a splice. Do not lint it.