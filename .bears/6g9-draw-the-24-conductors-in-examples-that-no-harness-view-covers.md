---
id: "6g9"
title: Draw the 24 conductors in examples/ that no harness view covers
status: open
priority: P2
created: "2026-11-20T23:07:40.066172618Z"
updated: "2026-11-20T23:07:40.066172618Z"
tags:
  - examples
  - manual
---

## Summary
The `undrawn_conductor` warning added by g43 fires 24 times on `examples/` (all on `Vehicle`), so `wirebug check --strict examples` now exits 1. They look like real gaps in the build drawings. Left for the project's author: it is the real vehicle project and the fix is a drawing decision.

## The gaps
- Ground rail in `systems/lv.wb`: `vcm.gnd`–`pump.gnd`–`fan.gnd`–`bl_left.gnd`–`bl_right.gnd`–`bl_chmsl.gnd`, plus `vcm.gnd` to `charger.gnd`, `inv.b_neg`, `front.gnd` (8 segments); `vcm.dcdc_en`–`charger.dc_sw`
- `systems/safety.wb`: `msd.interlock_out`→`front.interlock_in`, `rear.interlock_out`→`hvjb.interlock_in`
- `systems/cooling.wb`: PUMP PWM, PUMP TACH
- `systems/lighting.wb`: the STOP rail (3 segments)
- `looms/ac_charge.wb`: CP, PP
- Cab-side inline feeds in `brake_branch.wb`, `key_branch.wb`, `gear_branch.wb` (2 each): each loom draws only the male half; no view draws the female half with the VCM connectors

## Acceptance Criteria
- [ ] Each conductor is drawn by some harness view (e.g. a cab-side view with the VCM and the inline female halves; ground / lighting / cooling looms), or the gap is accepted knowingly
- [ ] `wirebug check --strict examples` passes again