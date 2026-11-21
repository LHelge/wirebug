---
id: xrs
title: Remove the deprecated named `text` form
status: open
priority: P3
created: "2026-11-21T02:48:31.134518859Z"
updated: "2026-11-21T02:48:31.134518859Z"
tags:
  - dsl
---

## Summary
k7r.xk made `text "<label>" at (x, y);` the text box syntax and kept the old named form (`text name at (x, y) "<label>";`) parsing with a `deprecated_text_syntax` warning, so the author's wiring project in another repository keeps rendering until it is migrated.

## Acceptance Criteria
- [ ] The wiring project (github.com/aphid-ev/wiring) no longer uses the named form
- [ ] The named form is a parse error with a help line showing the new form; `deprecated_text_syntax`, `duplicate_view_text` and the `data-text` SVG attribute are removed
- [ ] Docs, the skill and the TextMate grammar drop the old form

## Notes
- Embed hosts that style `[data-text=…]` lose that hook; say so in the release notes.