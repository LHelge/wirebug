---
id: k7r.64
title: Polish the `fragment` diagnostics
status: open
priority: P2
created: "2026-11-21T04:05:53.706072712Z"
updated: "2026-11-21T04:05:53.706072712Z"
tags:
  - dsl
  - check
---

## Summary
Left over from the second review round of k7r.fd (`fragment "<path>";`).

## Acceptance Criteria
- [ ] The docs and the code agree on which file is exempt from `unused_fragment` where it is named: the code (`merge_fragment` in `src/dsl/resolve/scopes.rs`, the `!target_ast.fragments.is_empty()` condition) exempts any file with at least one `fragment` line, the skill and the commit say a file whose only content is `fragment` lines. Pick one and make both say it
- [ ] Two files that `fragment` each other and add nothing are reported: `main.wb` has `fragment "systems/all.wb";`, a component and a view; `systems/all.wb` holds only `fragment "../main.wb";`. Today nothing is reported, even under `--strict`
- [ ] The `Empty` help is never false: `systems/all.wb` holding `use Vehicle from "a.wb"; fragment "a.wb";` with no `Vehicle` of its own, and `a.wb` holding `extend Vehicle`, says "`a.wb` has no view and no `extend`" (an `orphan_fragment` error comes with it)
- [ ] Decide whether `UnusedFragmentReason::ItsOwnFile` belongs under `reports_an_absence` (it is not an absence)


## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.