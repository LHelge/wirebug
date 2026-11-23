---
name: wirebug-dsl
description: "Use whenever editing, creating, reviewing, or discussing wirebug DSL files (.wb) — a text-defined language for electrical schematics and wiring harnesses: components, ports, connectors, wires, cables, splices, imports, views and grid layout. Trigger whenever the user mentions wirebug, .wb files, schematics, harnesses or wiring, designs an electrical system in text, or a .wb file is open, edited or referenced."
metadata:
  wirebug-version: "0.2.0"
---

# wirebug DSL

`.wb` files describe an electrical system: component types, their placements, the wires between them, and the views that draw them. This file is the whole language: write nothing it does not show; when something is awkward to express, ask rather than invent syntax.

Commands find the project by walking up to `wirebug.toml`, or take its directory or manifest:

```sh
wirebug init [dir]        # scaffold a project
wirebug check --strict    # report problems, warnings fatal: run after every edit
wirebug fmt [--check]     # format .wb files in place; --check only reports
wirebug render            # draw every view to <root>/out (--png, --pdf, --embed)
wirebug serve             # live-reloading preview
```

## Syntax

- `//` line comments only. A statement ends with `;`; a `{ }` block ends at `}` with **no** `;`, even in place of a statement's `;` (a wire body, `ports { }`).
- Lists in `[ ]` and `ports { }` side lists are comma-separated; a trailing comma is fine.
- Names: ASCII letters, digits, `_`, not starting with a digit; case-sensitive. A reference `inst.port` has at most two parts.
- Strings `"…"`: one line, no escapes; labels only, never references.
- Numbers: decimal, may be negative or fractional (`-2`, `0.5`, not `.5`); no arithmetic. Only a length takes a unit (see Wires), elsewhere it is a syntax error. A name touching digits is a unit: `cavity 1 at`, never `cavity 1at`.
- Convention (not enforced): types CamelCase, everything else snake_case. Ports: `_pos`/`_neg` power, `_p`/`_n` signal pairs, `_h`/`_l` differential. Labels short: `"HV+"`, `"CAN H"`.

**Reserved** words never name anything (`reserved_word`: rename `wire` to `wire_1`):

`use` `component` `extend` `connector_type` `connector` `inline` `port` `pub` `pin` `pins` `wire` `cable` `twisted` `view`

**Contextual** words are keywords only where their statement starts and ordinary names elsewhere (`pub port size "SIZE";`, an instance `grid: Sensor;`, an end `ecu.shield`):

`from` `fragment` `shield` `splice` `for` `include` `text` `at` `grid` `enclosure` `ports` `layout` `face` `cavity` `rows` `cols` `numbering` `size`

Not keywords at all: view kinds, property keys, `male`/`female`, sides, colors, numbering modes, `large`.

## Projects and files

A project is a directory with a `wirebug.toml` and the entry file `main.wb`, which defines exactly one top-level component, the design root (`no_root`).

```toml
[project]
name    = "my-project"   # required
version = "0.1.0"        # required; optional: description, authors = [..], license, revision, date
```

No other key. Drawings are stamped with name, version, revision and date; `revision` defaults to the git commit.

A file is loaded only when a `use` or `fragment` names it, starting from `main.wb`. Directories are for humans: hierarchy comes only from `use`, `fragment` and nesting. A file holds, in any order: `use`, `fragment`, `component`, `extend`, `connector_type`, `view`.

Conventional layout for a larger project:

```text
main.wb          # root component + overview view
systems/*.wb     # extend file per logical system: its instances, its signals' wires, its schematic views
looms/*.wb       # extend file per physical bundle: its multi-system cables and harness views
components/**    # one type per file; shared connector types in components/connectors.wb
```

Declare each instance in one file (its system's); a cross-system wire goes with the system owning the signal; a multi-system cable in its loom; a view beside the component it documents.

```wb
use Inverter from "components/inverter.wb";
fragment "systems/traction.wb";
```

- `use <Name> from "<path>";` loads the file (path relative to this file) and imports one top-level component or connector type. Nested definitions cannot be imported. `use_not_found` (no file), `unresolved_import` (no such name), `unused_import` (warning).
- `fragment "<path>";` loads a file without importing a name: its top-level `extend`s merge into the same-named component in scope here, its views render. To place a component it defines, `use` it. `unused_fragment` (warning) when the line adds nothing, its help says why; `duplicate_fragment` when one file names the same file twice.
- A file loads once however often it is named; cycles are fine.

### `extend`

```wb
// main.wb
use Battery from "components/battery.wb";
fragment "systems/traction.wb";

component Vehicle {
    pack: Battery "Battery";
}
```

```wb
// systems/traction.wb
use Inverter from "../components/inverter.wb";

extend Vehicle {
    inv: Inverter "Inverter";
    wire orange 50 "HV+" [pack.hv_pos, inv.dc_pos];   // `pack` is declared in main.wb
}
```

- The parts merge into one flat namespace: any part's wires and views use names from every part.
- Top level only (`nested_extend`). The component must be in scope where the file is pulled in (`orphan_fragment`, its help names the `use` to add): a file of only `fragment` lines pulling in further `extend` files needs `use Vehicle from "../main.wb";` (or an `extend` of its own).
- Each part `use`s the types it places. Part data is set once across all parts.
- `use Vehicle from "systems/traction.wb";`, naming the extended component, is an older spelling of `fragment`; it still works, but write `fragment`.

## Components

```wb
component BatteryPack {
    part:         "BP-96S";
    manufacturer: "Acme";
    description:  "96S traction pack";
    rating:       "400 V";

    component Stack {               // nested: only BatteryPack can place it
        pub port pos "+";
        pub port neg "-";
    }

    stack: Stack "Cell stack";

    pub port hv_pos "HV+";
    pub port hv_neg "HV-";
    port test "TEST";               // private

    wire orange 50 [hv_pos, stack.pos];
    wire orange 50 [hv_neg, stack.neg];
}
```

- A component is a type: `pub` ports are its interface, instances and wires its implementation. `duplicate_type`; a component placing itself is `containment_cycle`.
- `<name>: <Type> ["<label>"];` places an instance. The type is defined in this file, `use`d, or nested in the enclosing component (`undefined_type`). The label (default: the name) shows in drawings. Names unique per component (`duplicate_instance`).
- `[pub] port <name> "<label>" [pin N | pins [N, …]];` — the name is for wires and views, the label for drawings. Port names are unique across the component, connectors included (`duplicate_port`). Only `pub` ports are reachable from outside as `inst.port` (`private_port`); the component's own wires reach its ports by bare name.
- Visibility does not pass through levels: expose an inner port by wiring a `pub` port through (`hv_pos` above).
- Part data: `part`, `manufacturer`, `description`, `rating`, as `key: "string";` lines anywhere among the members. Each optional and set once (`duplicate_cable_property`), always a string (`cable_property_type`), no other key (`unknown_cable_property`). It belongs to the type; the bill of materials lists it, no view draws it. Put part numbers here, not in labels or comments. A type after the colon makes an instance: `part: Fuse;` places a `Fuse` named `part`.

## Connectors

A connector groups the ports in one housing and gives their pins. It is not a namespace (its ports are still `inst.port`); pins never change connectivity.

```wb
connector_type Dt06_6s "Deutsch DT06-6S" {
    part: "DT06-6S";
    layout grid { rows: 2; cols: 3; numbering: odd_even; }
}

component Ecu {
    connector x1 "Deutsch DT04-4P" {
        pub port b_pos "B+"  pin 1;
        pub port gnd   "GND" pins [2, 3];   // ganged for current
        port spare     "SPARE";             // pin not known yet
    }

    connector x2: Dt06_6s {
        pub port can_h "CAN H" pin 1;
        pub port can_l "CAN L" pin 2;
    }
}
```

- `connector <name> ["<description>"] { <port>* }` describes the housing in place; `connector <name>: <Type> { <port>* }` uses a connector type in scope (`undefined_connector_type`). The name is the **designator**, required and unique per component (`duplicate_connector_name`); views include it as `inst.x1`. The body holds only ports, `pub` and private mixed.
- Pins are positive integers (`invalid_pin`); a pin belongs to one port per connector (`duplicate_connector_pin`). A pin on a port outside any connector warns (`bare_port_pin`).
- `connector_type <Name> "<description>" { … }` is top level and `use`-importable; the description is required. Its body: `key: value;` lines (string or number; `part` is shown on drawings and the BOM, other keys are accepted but unused) and at most one layout. Without a layout a pinout view draws the table only.
- Layouts are seen **from the harness side** (wire entry): mirror a mating-face datasheet drawing.
- `layout grid { rows: N; cols: N; [numbering: <mode>;] }`, in this order. Modes: `row_major` (default: left to right, row by row), `odd_even` (column first: on 2×N odd pins on one row, even on the other), `clockwise` (on 2×N the upper row left to right, the lower mirrored), `counter_clockwise` (on 2×N the upper row mirrored, the lower left to right).
- `layout face { cavity <pin> at (x, y) [size large]; … }`, one or more cavities, for any other shape. Slot coordinates, `x` right, `y` down; `size large` fills 2×2 slots; the drawing starts at the first occupied slot.

```wb
connector_type InverterControl "Inverter control 6p" {
    layout face {
        cavity 1 at (0, 0) size large;
        cavity 2 at (2, 0) size large;
        cavity 3 at (4, 0);
        cavity 4 at (4, 1);
    }
}
```

### Inline connectors

```wb component
inline ic_pedal "Pedal branch" {
    male:   Dt04_3p;
    female: Dt06_3s;
    port apps1 "APPS1" pin 1;
    port gnd   "GND"   pin 3;
}
```

- `inline <name> ["<label>"] { … }`, a component member: a mated plug and receptacle where one loom hands over to another. One port set, two halves: pin N mates with pin N.
- `male: <Type>;` and `female: <Type>;` name each half's connector type; each optional, once (`duplicate_inline_half`), no other key (`unknown_inline_property`); halves and ports in any order.
- Ports take no `pub` (`inline_pub_port` warns): they are reachable anywhere in the component.
- It shares the instance namespace (`duplicate_instance`): wires end on `ic_pedal.apps1`. Both looms' conductors end on the same ports; that is the continuity.
- A harness view includes one half, the one built onto its loom (`include ic_pedal.female`); never both halves in one view, never in a schematic.

## Wires

```text
wire <color>[/<tracer>] <gauge> ["<label>"] [<end>, <end>, …];
wire <color>[/<tracer>] <gauge> ["<label>"] [<end>, <end>, …] { <key>: <value>; … }
```

```wb component
wire orange 50 "HV+" [pack.hv_pos, inv.dc_pos, conv.hv_pos];   // rail: all ends joined
wire green/yellow 2.5 "PE" [charger.pe, chassis];              // `chassis`: own port
wire red 0.5 "12V" [fuses.ecu, ecu.b_pos] {
    length: 150mm;
    type:   "FLRY-B";
    part:   "FLRY-B-0.5-RD";
}
```

- Color: IEC 60757 `black` `brown` `red` `orange` `yellow` `green` `blue` `violet` `grey` `white` `pink` `turquoise` `gold` `silver` (`purple`, `gray` mean `violet`, `grey`). Others still draw but warn (`unknown_wire_color`). Two-tone: base `/` tracer, `green/yellow`.
- Gauge: mm², positive, no unit (`invalid_gauge`). Label: optional signal name, shown in harness drawings.
- Ends: two or more (`wire_arity`), each once per wire (`duplicate_endpoint`): `inst.port` (a `pub` port, or an inline's port), or a bare name (an own port, or a splice). Two wires on one port: a double crimp.
- More than two ends is a shared rail (bus, ground, parallel coils on one enable): every end is joined and the branch point is left to the build. A physical joint is a `splice`; there is no other junction element.
- Body, in place of `;`: `length`, `type: "<string>";`, `part: "<string>";`, each optional and once. Other keys `unknown_cable_property`, repeats `duplicate_cable_property`, wrong kind `cable_property_type`. On a rail `length` is the whole rail's.
- Length: positive (`invalid_length`); bare is metres; units `m`, `cm`, `mm` written directly after the number (`150mm`; `2 m` is a syntax error; others `unknown_length_unit`).

## Cables

```wb component
cable can2 "CAN2" {
    type:   "Shielded twisted pair";
    length: 1.6;

    wire red 1.5 "12V" [ecu.pwr, rear.pwr] { length: 180cm; }
    twisted {
        wire white/blue 0.5 "CAN H" [rear.can_h, vcm.can_h];
        wire white/red  0.5 "CAN L" [rear.can_l, vcm.can_l];
        shield [vcm.shield_gnd];
    }
    shield "SHLD" [vcm.shield_gnd, rear.shield_gnd];
}
```

- `cable <name> ["<label>"] { … }`, a component member: conductors that run together; harness views draw it as a box titled with the label (default: the name). Names unique per component (`duplicate_cable_name`).
- Body, in any order: `type: "<string>";`, `length: <length>;` (only these two keys; `part` is per wire), conductors, `twisted` pairs, at most one `shield`.
- A conductor is a `wire` with exactly two ends (`cable_wire_arity`); a rail stays a loose wire. A conductor's own `length` overrides the cable's. No conductor: `empty_cable` (warning). Cable wires are ordinary connections, drawn in schematics too.
- `twisted { <wire> <wire> [<shield>] }`: exactly two wires, else a syntax error.
- `shield ["<label>"] [<end>, …];` lists the pins its drain is terminated on: one (the usual) or two; `shield ["<label>"];` lands on no pin (cut back, or bonded at a gland: say which in a comment). More than two `shield_arity`; `[]` is a syntax error. It is no conductor: no color or gauge, not counted as a wire. At most one per cable body and per pair (`duplicate_shield`). A drain may share a pin with a wire, never a splice (`shield_on_splice`). Never model a screen as a `wire`.

## Splices

```wb component
splice sp_gnd "Ground splice" {
    part:        "D-436-36";
    description: "Solder sleeve";
}
splice sp_butt;

wire black 1   "GND" [ecu.gnd,  sp_gnd];
wire black 0.5 "GND" [lamp.gnd, sp_gnd];
wire black 2.5 "GND" [sp_gnd, bat.neg];
```

- `splice <name> ["<label>"];`, or with a body of `part` and `description` (strings, once each) for the `;`, is a component member: a crimp or solder joint. Use it when legs differ in color, gauge or length, or the joint is a part.
- Wires end on it by bare name. The name must differ from every port, instance, inline, connector, cable and splice of the component (`duplicate_splice_name`); every `extend` part sees it.
- Every conductor ending on a splice is joined, splice-to-splice wires included.
- A wire to a splice has exactly two ends (`splice_wire_arity`). Fewer than two conductors: `splice_joins_nothing` (warning); two is a butt splice. A bare end that is neither port nor splice: `unknown_port`.

## Views

```text
view <schematic|harness|pinout> "<title>" [for <Component>] { … }
```

- Top level; other kinds `unknown_view_kind`. Titles unique per project (`duplicate_view_title`, warning). Body, in any order: at most one `grid: <n>;` and one `enclosure { }` (`duplicate_view_item`), any number of `include`s and `text`s.
- The **subject** is the component the view documents; every include resolves against it. Without `for` it is the file's single top-level component (in an `extend` file, the extended one); with none or several, `for` is required (`view_subject`). `for` names a top-level component defined or `use`d in the file (`undefined_type`; a connector type `view_subject_not_component`). The subject must be placed in the design (`uninstantiated_view_subject`).
- `grid:` is drawing units per cell, positive (`invalid_grid`), at least 7.5 in a schematic (`grid_too_fine`). Every position counts grid cells (negative is fine), never pixels.
- Includes may touch but not overlap (`overlapping_includes`), each target once (`duplicate_view_include`). A drawing spans at most 8192 units per side (`view_too_large`): split it.
- Views never list wires: they draw those between what they show.

### Schematic

```wb view
grid: 20;

enclosure {
    hv_pos at (west, 4);
    sense  at (2, north);
}

include pack at (2, 5) ports {
    east: hv_pos, hv_neg;
}
include inv at (20, 2) ports {
    west:  dc_pos, dc_neg;
    south: enable;
}
include conv at (20, 18);

text "120 Ω termination" at (8, 4);
```

- `include <instance> at (x, y) [ports { <side>: <port>, …; … }]` centres the box on the point; `;` only without `ports`. Instances only (an inline or splice is `wrong_include_form`).
- Sides `north` `east` `south` `west` (`unknown_port_side`). Order: top to bottom on west and east, left to right on north and south; repeated side lines join. Only `pub` ports, once per include (`duplicate_view_port`).
- The listing is the scope: unlisted ports are hidden (no `ports` block draws a bare box), and a wire is drawn only between listed ports or enclosure ports. A listed port wired to something not shown gets a stub naming the far end (`→ vcm.can_h`, `→ sp_gnd (lamp.gnd, +1 more)`). No splice or inline is drawn; the ports they join show connected.
- Sides are authored, never guessed: put a port on the side facing what it wires to, and align boxes so wires run straight. Ports sit two grid steps apart; a wired port needs two clear steps before the box it faces (`include_too_close`); a walled-in wire is `unroutable_wire`. Leave a box's title row free. Pins show beside ports, `x1:2` with several connectors: keep designators short.
- `enclosure { <port> at (<x>, <y>); … }` draws the subject as a dashed box around the includes, with its own `pub` ports on the edge (`unknown_port`, `private_port`, each once `duplicate_view_port`). One coordinate is a side (`west`/`east` as x, `north`/`south` as y), the other a grid position along that edge (`enclosure_anchor`). An own port it does not place gets a `→ <port> (leaves <Subject>)` stub; `enclosure { }` draws the box alone.
- `text "<label>" at (x, y);` is a boxed note centred on the point; schematic only (`unsupported_view_text`), not overlap-checked. The old `text <name> at (x, y) "<label>";` warns (`deprecated_text_syntax`; a repeated name `duplicate_view_text`): write the label first.

### Harness

```wb view
include ecu.x1          at (0, 0);
include bms.x2          at (30, 0);
include sp_gnd          at (15, 10);
include ic_pedal.female at (30, 12);
```

- A target is `<instance>.<connector>` (`unknown_connector`), `<inline>.male` or `<inline>.female` (a declared half, `undeclared_inline_half`; one half per inline per view), or a bare splice, drawn as a dot. No `ports` block (`wrong_include_form`) and no sides: a pin table shows only the pins carrying a conductor in the view, oriented toward its cables.
- A conductor is drawn when both its ends land on included connectors or splices; a cable's run through its box. Ends on connectorless or own ports are not drawn: put a port a harness should show in a `connector`.
- Once a component has a harness view, every conductor between its children's connectors, or to its splices, must appear in one of them (`undrawn_conductor`, warning). A rail counts per segment (`[a, b, c]` is `a–b`, `b–c`); an inline counts when either half is included.

### Pinout

```wb view
include hv      at (0, 0);
include control at (14, 22);
```

- A target is a connector of the subject itself (`include control`, not `include inv.control`); no `ports` block. It draws the face from the connector type's layout, seen from the harness side, above a crimp table of each pin's port and conductors. Keep cavity labels short.
