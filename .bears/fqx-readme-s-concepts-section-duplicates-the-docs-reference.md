---
id: fqx
title: README's Concepts section duplicates the `docs/` reference
status: open
priority: P3
created: "2026-11-21T21:00:04.242116248Z"
updated: "2026-11-21T21:00:04.242116248Z"
tags:
  - docs
---

## Summary
After g2e.f7, `docs/` is the language reference, but README's Concepts section (about lines 150–205) still covers syntax and diagnostic codes in untagged code blocks that `tests/docs.rs` does not check. No contradiction today; a drift risk.

## Acceptance Criteria
- [ ] README keeps a short tour and links to `docs/` for the language, or its `wb` blocks are tagged so `tests/docs.rs` checks them
- [ ] No language rule is stated only in README