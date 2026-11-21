---
id: jkx.cp
title: Schematic rendering is not deterministic
status: open
priority: P1
created: "2026-11-21T04:05:53.694934639Z"
updated: "2026-11-21T04:05:53.694934639Z"
tags:
  - render
  - schematic
---

## Summary
Found while reviewing k7r.fd. Rendering `examples/` twice does not always give the same bytes: `cooling_system.svg` comes out two ways (wire points such as `360.00000002,240` versus `640,240`, and a viewBox 716 versus 736 wide), the minority output in roughly 1 run of 10. It predates the DSL epic and happens with either import spelling, so it looks like hash-iteration order or an unstable tie-break in schematic routing (`src/render/schematic/route/`).

## Acceptance Criteria
- [ ] The cause is found (a `HashMap`/`HashSet` iteration reaching the output, or an A* / nudging tie-break) and removed
- [ ] Rendering any fixture N times in one test gives identical SVG every time; the test fails on the code before the fix
- [ ] The same holds across processes (hash seeds differ per process): a CLI test renders a fixture twice and compares
- [ ] `check`'s layout lints and `render` still agree (the lints run the same layout)


## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.