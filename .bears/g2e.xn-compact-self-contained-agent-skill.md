---
id: g2e.xn
title: Compact, self-contained agent skill
status: done
priority: P2
created: "2026-11-20T22:44:45.081803839Z"
updated: "2026-11-21T20:29:23.897381029Z"
tags:
  - docs
  - skill
depends_on:
  - g2e.f7
  - g2e.6x
attempts: 1
---

## Summary
The `wirebug-dsl` skill is 604 lines / 30 KB, loaded into an agent's context whenever a `.wb` file is touched. It carries design rationale, repeated examples and a "Do not" list that restates earlier rules, and it assumes it is read inside this repository (`cargo run -- check`, "conventions the `examples/` project follows"). It is about to ship to other repositories, so it has to be compact and stand on its own.

## Decided (2026-11-20)
- Two hand-written references for two readers: `docs/` (c94) is written for a human, the skill is written to be context-efficient for an agent. Neither is generated from the other.
- The skill stays a single `SKILL.md` — no `references/` split — so it can be printed to stdout and installed as one file.
- Drift between the two is caught by tests, not by review.

## Acceptance Criteria
- [ ] Every construct the parser accepts is documented in the skill, checked against the c94 reference construct by construct
- [ ] Every code block in the skill parses, using the same extract-and-check test c94 adds for `docs/`
- [ ] A test fails when a language keyword appears in `docs/` but not in the skill, or the reverse
- [ ] No reference to this repository: commands are written as `wirebug check` (not `cargo run --`), and layout conventions are stated as conventions, not as "what `examples/` does"
- [ ] Rationale prose, duplicated examples and rules restated in "Do not" are gone; a rule is stated once, where the construct is introduced
- [ ] At most 20 KB (30 KB today); the PR reports before and after size, and names any content that resisted compression
- [ ] The frontmatter `description` still triggers on `.wb` files and wirebug topics

## Implementation Notes
- Depends on c94 so the accurate construct list and the code-block test exist first, and on yyd, which already rewrites the skill description and intro generically.
- Prefer grammar-style one-liners and tables over paragraphs (port and pin forms, naming conventions, view options).
- Keep the "Common patterns" only where a pattern is not obvious from the construct itself.
- The file stays at `.claude/skills/wirebug-dsl/SKILL.md` in this task; the follow-up task moves the source.

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.