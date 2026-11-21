---
id: g2e.ty
title: Make examples/ pass `check --strict` and check it in CI
status: open
priority: P2
created: "2026-11-20T23:07:40.066172618Z"
updated: "2026-11-20T23:26:27.405796018Z"
tags:
  - examples
  - ci
depends_on:
  - g2e.6x
---

## Summary
`examples/` is no longer the author's vehicle project (that moved to github.com/aphid-ev/wiring); it is wirebug's generic example and should be a clean one. Today `wirebug check --strict examples` exits 1: the `undrawn_conductor` warning (g43) fires 24 times, all on `Vehicle`, and nothing in CI would notice a regression.

## The gaps
- Ground rail in `systems/lv.wb`: `vcm.gnd`–`pump.gnd`–`fan.gnd`–`bl_left.gnd`–`bl_right.gnd`–`bl_chmsl.gnd`, plus `vcm.gnd` to `charger.gnd`, `inv.b_neg`, `front.gnd` (8 segments); `vcm.dcdc_en`–`charger.dc_sw`
- `systems/safety.wb`: `msd.interlock_out`→`front.interlock_in`, `rear.interlock_out`→`hvjb.interlock_in`
- `systems/cooling.wb`: PUMP PWM, PUMP TACH
- `systems/lighting.wb`: the STOP rail (3 segments)
- `looms/ac_charge.wb`: CP, PP
- Cab-side inline feeds in `brake_branch.wb`, `key_branch.wb`, `gear_branch.wb` (2 each): each loom draws only the male half; no view draws the female half with the VCM connectors

## Acceptance Criteria
- [ ] Every conductor is drawn by some harness view (e.g. a cab-side view with the VCM and the inline female halves; ground / lighting / cooling looms); the lint is not narrowed and no wire is deleted to get there
- [ ] `wirebug check --strict examples` exits 0 and `wirebug render examples` renders every view
- [ ] CI (`.github/workflows/quality-checks.yml`, shared by `ci.yml` and `release.yml`) runs `check --strict examples` and fails on any error or warning
- [ ] `AGENTS.md` no longer calls `examples/` the real vehicle project: it is the generic example, kept passing by CI. Cargo tests still assert on `tests/fixtures/`, never on `examples/` content

## Decided (2026-11-21, by the author)
`examples/` should no longer reflect the author's project; it must pass `check`, and CI must run that check. Orchestrator's reading: `--strict`, so warnings fail CI too.

## Testing
- `cargo fmt && cargo clippy -- -D warnings && cargo test` pass.