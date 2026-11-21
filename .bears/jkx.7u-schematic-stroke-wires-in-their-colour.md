---
id: jkx.7u
title: "Schematic: stroke wires in their colour"
status: done
priority: P1
created: "2026-11-20T21:40:32.046543061Z"
updated: "2026-11-21T12:22:38.030091735Z"
tags:
  - render
  - schematic
attempts: 1
---

## Summary
Every schematic wire is black; colour is only a tiny two-letter code, unreadable once a wide view is fitted to A4.

## Acceptance Criteria
- [ ] Segments are stroked with `wire.color` over a dark casing (the harness renderer's technique) so light colours read
- [ ] Two-tone wires show the tracer
- [ ] The `wire-code` annotation stays
- [ ] Embed mode exposes colour via `data-color`, as the harness does

## Implementation Notes
- `src/render/schematic/draw.rs`; share the casing/tracer drawing with `render/harness/draw.rs` instead of copying it
- AGENTS.md lists "theming, colour" as out of scope: this is wire identity, not theming — update that note

## Testing
- Unit tests beside the code; end-to-end coverage via `tests/fixtures/` (never `examples/`).
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.