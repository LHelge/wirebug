---
id: g2e.h6
title: "`wirebug skill` prints or installs the agent skill"
status: done
priority: P2
created: "2026-11-20T22:44:45.099268939Z"
updated: "2026-11-21T20:48:51.082542953Z"
tags:
  - cli
  - skill
depends_on:
  - g2e.xn
attempts: 1
---

## Summary
A repository that uses wirebug has no way to get the DSL skill other than copying it out of this repo by hand, and a copy goes stale when wirebug is upgraded. The binary should carry the skill that matches its own version and be able to print or install it — typically run by an agent.

## Decided (2026-11-20)
- `wirebug skill` prints the skill to stdout. This is the default: it writes nothing, composes with redirection, and lets an agent read the reference without installing it.
- `wirebug skill --install <harness>` writes it where that harness expects it, creating directories. Only `claude` exists now (`.claude/skills/wirebug-dsl/SKILL.md`); the value enum is the extension point, no other harness is pre-baked.
- No destination-path argument: redirection covers it, and the caller would still have to know the right path.
- The canonical source moves out of `.claude/` to a harness-neutral `skill/SKILL.md`.

## Acceptance Criteria
- [ ] The skill source lives at `skill/SKILL.md` and is embedded with `include_str!`
- [ ] `skill/**` is in the `Cargo.toml` `include` list (it is a whitelist and has no `.claude/` entry today); `cargo package --list` shows the file and `cargo publish --dry-run` builds
- [ ] `wirebug skill` writes the skill to stdout and nothing else, exit 0
- [ ] `wirebug skill --install claude` writes `.claude/skills/wirebug-dsl/SKILL.md` under the current directory, creates missing directories, overwrites an existing file, and prints the path written
- [ ] The install path is relative to the current directory, not the `wirebug.toml` root, and works with no wirebug project present
- [ ] The emitted skill names the wirebug version that produced it, without breaking the frontmatter
- [ ] This repository's `.claude/skills/wirebug-dsl/SKILL.md` is an install output; a test fails when it differs from the source (ignoring the version line)
- [ ] A write failure is a concrete `error::Error` variant naming the path
- [ ] README gains a short "Using wirebug with a coding agent" section; `AGENTS.md`, `ARCHITECTURE.md` (One pipeline, Source tree) and the `--help` snapshot are updated

## Implementation Notes
- `Command::Skill { install: Option<Harness> }` in `src/cli.rs`; `Harness` is a `clap::ValueEnum` whose method returns the install path.
- The code-block and keyword-coverage tests from the previous task must follow the file to `skill/SKILL.md`.
- The version line must not make the committed install output a third file to bump at release — hence the sync test ignoring it.

## Testing
- `assert_cmd` tests: stdout form matches the embedded source; `--install claude` in a temp dir creates the tree, and a second run overwrites cleanly.
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.