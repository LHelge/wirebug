---
id: g2e.6x
title: Remove Aphid EV naming from examples and docs
status: open
priority: P2
created: "2026-11-20T21:50:43.111102249Z"
updated: "2026-11-20T21:50:43.111102249Z"
tags:
  - docs
  - examples
---

## Summary
The Aphid EV project moved to its own repo, but `examples/` and the docs still carry its name: `examples/wirebug.toml` is `aphid-evpack` by "Aphid EV team", most files under `examples/` open with an `// Aphid EV — …` comment, the README manifest sample uses the same name, and both copies of the DSL skill say the language is "for the Aphid EV project".

## Acceptance Criteria
- [ ] `examples/wirebug.toml` has a generic name, description and authors — decided 2026-11-20: name `ev-conversion-example`, description "Example EV conversion — vehicle wiring"
- [ ] Header comments in `examples/**/*.wb` no longer mention Aphid
- [ ] README manifest sample updated to match
- [ ] Skill description and intro in `.claude/skills/wirebug-dsl/SKILL.md` describe wirebug generically
- [ ] Test data in `src/render/mod.rs` that uses the Aphid name is renamed too
- [ ] `grep -ri aphid` over the repo (excluding `.bears/` and git history) returns nothing

## Implementation Notes
- The PDF export is named after the project slug, so the rendered file name changes with the manifest name
- `bacon.toml` and CI reference `examples/` by path, not by project name — verify, no change expected

## Testing
- `wirebug check --strict examples` still passes
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass (tests never read `examples/`)