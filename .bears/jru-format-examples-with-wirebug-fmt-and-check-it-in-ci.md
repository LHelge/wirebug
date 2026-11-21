---
id: jru
title: Format `examples/` with `wirebug fmt` and check it in CI
status: open
priority: P3
created: "2026-11-21T20:24:03.273154593Z"
updated: "2026-11-21T21:00:04.231100362Z"
tags:
  - examples
  - ci
---

## Summary
`wirebug fmt` (g2e.nu) would reformat 29 of the 39 files in `examples/` (184 lines, mostly alignment the examples apply unevenly). Once the house style has settled, format them and gate `wirebug fmt --check examples` in `.github/workflows/quality-checks.yml`.

## Acceptance Criteria
- [ ] `wirebug fmt --check examples` exits 0
- [ ] CI runs it beside `check --strict examples`
- [ ] `check --strict examples` still passes
- [ ] The `wb` blocks of `docs/` and `skill/SKILL.md` are fmt-clean (7 of 26 are not today: aligned trailing comments, `rows:`/`cols:` columns, endpoint columns), and `tests/docs.rs` asserts it for whole-file blocks; the skill stays within its 20,000-byte budget