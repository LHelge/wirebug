---
name: wirebug-dsl
description: Use this skill whenever editing, creating, reviewing, or discussing wirebug DSL files (.wb extension) — a text-defined electrical schematic and wiring harness language for the Aphid EV project. Covers component definitions, type/instance separation, ports (pub vs private), connector groupings, multi-endpoint wires, splices, hierarchical composition, imports between files, views, and grid-based layout. Trigger whenever the user mentions wirebug, .wb files, components, schematics, harnesses, EV wiring, or is designing electrical systems in text. Trigger whenever a .wb file is open, being edited, or referenced.
---

# wirebug DSL

wirebug is a text-defined DSL for describing electrical systems — components, wires, and the views that render them. Source files end in `.wb`. The DSL was designed for the Aphid EV conversion project but the syntax is general-purpose.

The syntax here is authoritative and matches the implemented parser (`cargo run -- check <project>` validates a project). Avoid inventing syntax that isn't documented here or already present in the project's existing `.wb` files — when in doubt, ask.

The surface syntax deliberately borrows Rust's feel: `use` imports, `pub` visibility, `//` comments, `;`-terminated statements, brace blocks with no trailing `;`, `name: Type` instantiation, and CamelCase type names against snake_case value names.

## Project structure

A wirebug project is a directory rooted at a `wirebug.toml` manifest, with `main.wb` as the conventional entry file beside it. The presence of `wirebug.toml` identifies a directory as a wirebug project; CLI commands follow Cargo's shape by walking up from the current directory until they find the manifest. Commands may also point directly at the project root or at `wirebug.toml`.

A conventional layout, which is **convention, not requirement**:

```
my-ev/
├── wirebug.toml         # project metadata and root marker
├── main.wb              # near-empty root component + whole-vehicle overview view
├── systems/             # one `extend`-fragment per LOGICAL system: its
│   ├── traction.wb      #   instances, the wires of the signals it owns,
│   ├── safety.wb        #   and its schematic view(s)
│   └── ...
├── looms/               # one fragment per PHYSICAL harness bundle: the
│   ├── tunnel.wb        #   multi-system cables that travel together and
│   └── ...              #   the harness views (the build documents)
└── components/          # imported components, one type per file,
    ├── connectors.wb    #   subfoldered by domain; shared connector_type
    ├── battery/         #   library in connectors.wb
    ├── hv/
    └── lv/
```

Conventions the `examples/` project follows: every instance is declared in exactly one fragment (the system the device belongs to); cross-system wires live in the fragment owning the *signal* (interlock wires in `safety.wb`, 12V and grounds in `lv.wb`); a single-system point-to-point cable stays in its system file, a multi-system bundle lives in its loom file; views sit in the file whose subject they document (component detail views beside the component, system views in the system fragment, harness views in the loom).

Filesystem layout is for human navigation only. **A component's logical position in the hierarchy is determined entirely by `use` and `fragment` statements and nesting in the DSL**, not by what directory the file lives in. Moving a `.wb` file to a different folder must never change the model — the only thing that matters is which paths the `use` and `fragment` statements resolve to.

## File contents

A `.wb` file contains zero or more of, in any order:

- `use` declarations (imports from other files)
- `fragment` statements (files pulled in for their `extend`s and views)
- `component` definitions (type definitions, which may nest other definitions)
- `view` declarations (rendering targets)

Comments are `//` line comments only. Block comments are not supported.

Statements end with `;`; brace blocks end at the closing brace with no trailing `;` (so a wire with a property body ends at its `}`). Comma-separated lists — `pins [ ]`, wire endpoint lists, and view `ports` side lists — allow a trailing comma, so multi-line lists diff cleanly. Numbers may be negative (`at (-2, 4)`); there is no other arithmetic.

## Imports

```
use CellModule from "cell_module.wb";
use Contactor  from "components/contactor.wb";
```

Paths are relative to the importing file. A `use` brings one named top-level component definition into the current file's scope. Only top-level definitions can be imported — definitions nested inside other components are private to their parent.

If multiple imports come from the same file, repeat the statement (no list form yet):

```
use CellModule from "components/cell_module.wb";
use Contactor  from "components/contactor.wb";
```

### Pulling in a file (`fragment`)

```
fragment "systems/traction.wb";
```

A `fragment` statement loads a file into the project **without importing any name**. The path is relative to the file that names it, as in a `use`. It is the statement for a file of `extend` fragments and views (see Splitting a component across files): each top-level `extend` in that file merges into the component of the same name in this file's scope, and its views render. A component the pulled-in file defines is *not* in scope here — to instantiate it, `use` it by name.

- A file loads once however many `use` and `fragment` lines name it, so naming one file in both is fine, and so is a cycle.
- A missing file is `use_not_found`, labelled on the path.
- A `fragment` that adds nothing warns (`unused_fragment`, fatal under `--strict`): a stale line. A line adds something when it merges one of the file's `extend`s, or when the file contributes on its own: it has a view (and no `extend` that merged into nothing), a `use` or `fragment` line of its own that merges an `extend`, or a `fragment` line naming a file that contributes. A file already loaded on the way (the project's entry file, the file the line is written in, the files in between) contributes nothing again, so two files that only name each other are both reported, and so is a line naming the entry file that merges nothing. The help says why: the file has no view and no `extend`; its own `fragment` lines add nothing either; its `extend`s name components not in this file's scope (the help names them); the component in scope is that same `extend`, imported with `use` (import the component itself); it is the entry file; or it is the file the line is written in. An aggregating file that `use`s the component and holds `fragment` lines that merge into it is fine, both where it is named and in itself.
- Two `fragment` lines in one file naming the same file warn (`duplicate_fragment`); remove the repeat.

## Component definitions

A component is a **type**, not an instance. It defines an interface (its `pub` ports) and an implementation (instances of other types, plus wires connecting them).

A minimal primitive component (no internals, just an interface):

```
component Contactor {
    pub port in     "IN";
    pub port out    "OUT";
    pub port coil_p "COIL+";
    pub port coil_n "COIL-";
}
```

Components can nest. A definition inside another component is scoped to its parent and is not exported — it can only be instantiated by code inside the containing definition:

```
component CellModule {
    component CellPack {        // private to CellModule
        pub port hv_pos "+";
        pub port hv_neg "-";
        // ...
    }

    pack: CellPack;             // instantiate the nested type

    pub port hv_pos "HV+";      // external interface of CellModule
    pub port hv_neg "HV-";

    wire orange 50 [hv_pos, pack.hv_pos];   // external port wired to internal
    wire orange 50 [hv_neg, pack.hv_neg];
}
```

### Part data

A component body may carry `key: value;` lines that say what the type is bought as — what a bill of materials lists it by. They sit anywhere among the members:

```
component Contactor {
    part:         "EV200AAANA";
    manufacturer: "TE Connectivity";
    description:  "HV main contactor, 12 V coil";
    rating:       "500 A";

    pub port in  "IN";
    pub port out "OUT";
}
```

- `part: "<string>";` — the manufacturer part number.
- `manufacturer: "<string>";` — who makes it.
- `description: "<string>";` — what it is, for humans.
- `rating: "<string>";` — free text such as `"500 A"` or `"12 V, 2 W"` (a string, since ratings mix units).

All four are optional, each may appear once, and every value is a quoted string. The data belongs to the **type**: every placement of `Contactor` carries it. A type without a `part` is still fine, just one a bill of materials cannot name. Any other key is an error (`unknown_cable_property`, the code cable, wire and component bodies share), as is a key set twice (`duplicate_cable_property`) or a number where the string belongs (`cable_property_type`). Part data is not drawn in any view.

A `<name>:` line is part data only when a **string or number** follows the colon; a type name after the colon makes it an instance, so `part: Fuse "F1";` places a `Fuse` named `part`.

## Splitting a component across files (`extend`)

A large top-level component — the vehicle root, say — can be authored across
several files. One file introduces it with `component`; others add to it with
`extend`. `main.wb` pulls the fragments in with `fragment` statements:

```
// main.wb
fragment "traction.wb";
fragment "charging.wb";
use Battery from "components/battery.wb";

component Vehicle {
    pack: Battery "Battery";         // shared HV battery lives here
}
```

```
// traction.wb
use Inverter from "components/inverter.wb";

extend Vehicle {
    inv: Inverter "Inverter";
    wire orange 50 "HV+" [pack.hv_pos, inv.dc_pos];   // pack is from main.wb
    wire orange 50 "HV-" [pack.hv_neg, inv.dc_neg];
}
```

Rules:

- The fragments merge into one component. `component Vehicle` (in `main.wb`) is
  the **root fragment**; each `extend Vehicle` adds members. `main.wb` must
  still declare exactly one top-level `component`.
- `fragment "traction.wb";` loads the file, and its `extend Vehicle` merges
  into the `Vehicle` in `main.wb`'s scope. A file of fragments may pull in
  further fragment files the same way, as long as it has the component in its
  own scope: its own `extend Vehicle` counts, and a file of nothing but
  `fragment` lines gets it with `use Vehicle from "../main.wb";`. Merging
  does not pass through files: without that `use`, the `extend`s it pulls in
  merge into nothing, each an `orphan_fragment` error whose help names the
  `use` to add and where. Their lines, and the line naming the aggregating
  file, also warn (`unused_fragment`), even when the pulled-in files have
  views, since those views document a component they never joined.
- `use Vehicle from "traction.wb";` is the older spelling of the same: it
  loads the file and triggers the merge (a same-name collision with `extend`
  on either side merges instead of erroring). It keeps working, with no
  warning; prefer `fragment` in new files, since it says what it does.
  Without `extend`, two same-named `component`s are still a duplicate-type
  error.
- Each fragment carries its **own** `use` imports for the types it
  instantiates (`traction.wb` imports `Inverter` itself).
- The merged component is **one flat namespace**: a wire or view in any
  fragment may reference an instance or port declared in any other fragment
  (`traction.wb` wires to `pack`, declared in `main.wb`). Views always see the
  whole merged component.
- `extend` is top-level only (not nested), and every `extend <name>` needs a
  `component <name>` somewhere — a lone `extend` is an error.
- Part data (`part:`, `manufacturer:`, …) is set **once** across the merged
  component: in the `component` or in one `extend`. An `extend` may set a key
  the `component` left unset; setting a key that the `component` or another
  `extend` already set is an error (`duplicate_cable_property`, pointing at
  both places).

## Ports

```
port name "Label";          // internal port — only visible inside this component
pub port name "Label";      // external port — visible to instantiators
```

Every port has a name (snake_case identifier, used in code) and a quoted label (human-readable, used in diagrams). They can differ — name is for the tool, label is for the eye.

### Pin assignments

Ports inside a `connector` block can declare which physical pin(s) they map to. This is metadata for the harness renderer and BOM — it does not affect wiring logic.

```
port name "Label" pin N;            // single pin
port name "Label" pins [N, N, N];   // multiple pins (e.g. ground or high-current)
```

Pin numbers are positive integers. A port may span multiple pins when several connector pins are ganged for current capacity (common for power and ground). Pin assignment is optional — omit it when the physical pin-out is unknown or irrelevant.

Example:

```
connector "Molex MX-150 12p" {
    pub port b_pos "B+" pin 1;
    pub port gnd   "GND" pins [2, 3, 4];
    pub port can_h "CAN H" pin 5;
    pub port can_l "CAN L" pin 6;
}
```

**Visibility does not propagate automatically.** If `CellPack` declares `pub port hv_pos`, that port is visible when `CellPack` is instantiated — but the containing `CellModule` does *not* automatically expose it. To re-export, the parent declares its own `pub port` and wires it through:

```
component CellModule {
    pack: CellPack;
    pub port hv_pos "HV+";                  // explicit external port
    wire orange 50 [hv_pos, pack.hv_pos];   // wired to inner port
}
```

This is more verbose than implicit propagation but does useful work: documents the abstraction boundary, allows renaming across boundaries (inner `hv_pos`, outer `hv_p`), prevents accidental leaks.

## Connector blocks

A `connector` block groups ports that physically belong to a single connector part (a JST XH 15p, a Deutsch DT06-12S, etc.). The block carries a **required designator** (a snake_case reference name), an **optional description** string for the physical part, and contains port declarations with optional pin assignments. (Manufacturer part numbers belong on a `connector_type`'s `part:` property, not in the description string.)

```
component CellMonitor {
    connector cells "JST XH 15p" {       // `cells` is the designator
        port c0 "C0" pin 1;
        port c1 "C1" pin 2;
        // ... c2..c12
        port ntc_p "NTC+" pin 14;
        port ntc_n "NTC-" pin 15;
    }

    connector iso_spi "JST XH 2p" {
        pub port iso_spi_p "ISO SPI+" pin 1;
        pub port iso_spi_n "ISO SPI-" pin 2;
    }
}
```

The designator addresses the connector in harness and pinout views (`include <inst>.<designator>`, see Views). Designators must be unique within a component. Like every display string in the language, the description is optional and identifiers are required — same shape as `cable <name> ["<label>"] { }`.

Connectors are **structural metadata, not a namespace**. A port `c0` inside a connector is still referenced as `cell_monitor_instance.c0`, not `cell_monitor_instance.cells.c0` — the designator names the *connector*, not a port scope. The `connector` block carries physical-grouping info (including pin assignments) for the harness renderer and BOM. `pub` is independent — a connector can mix `pub` and non-`pub` ports freely.

## Connector types and pinout layouts

Reusable connector types keep verbose connector metadata and physical pinout
layouts out of component definitions. A component instantiates a connector
type with the **same body as an inline connector** — port declarations with
their pin assignments — so the two forms differ only in where the part
metadata comes from (`: Type` versus a free part string).

```
connector_type JstXh8p "JST XH 8p" {
    part: "B8B-XH-A";

    layout grid {
        rows: 1;
        cols: 8;
        numbering: row_major;
    }
}

component Controller {
    connector x1: JstXh8p {
        pub port can_h "CAN H" pin 1;
        pub port can_l "CAN L" pin 2;
    }
}
```

`connector_type` definitions are top-level items, like components and views.
They can be imported with `use` and referenced by `connector <name>: <Type>`.
Connector instance names are the designators used in harness and pinout views.
Ports declared inside the block are ordinary flat component ports, exactly as
in an inline connector. Pins must be positive integers; a pin can belong to
only one port within a connector, but one port may span several pins with
`pins [1, 2]` when cavities are ganged for current.

Pinout layouts are authored from the **harness side**. Be explicit about this
in project docs and connector libraries: if a datasheet drawing is device-side,
mirror it before entering the layout.

Simple rectangular connectors use a grid:

```
connector_type Linear8p "Linear 8p" {
    layout grid {
        rows: 1;
        cols: 8;
        numbering: row_major;
    }
}

connector_type Dual16p "Dual row 16p" {
    layout grid {
        rows: 2;
        cols: 8;
        numbering: odd_even;
    }
}
```

Supported grid numbering modes:

- `row_major` — left-to-right across each row, then the next row.
- `odd_even` — column-first, useful for paired 2xN connectors.
- `clockwise` — walks around the face clockwise.
- `counter_clockwise` — walks around the face counter-clockwise; for a 2xN
  connector the upper row is mirrored and the lower row reads left-to-right.

Complex connectors use an explicit face layout:

```
connector_type InverterControl "Inverter control 47+13p" {
    layout face {
        cavity 47 at (1, 0) size large;
        cavity 46 at (3, 0) size large;
        cavity 49 at (1, 2) size large;
        cavity 48 at (3, 2) size large;

        cavity 21 at (5, 0);
        cavity 20 at (6, 0);
        cavity 19 at (7, 0);

        cavity 2 at (17, 0);
        cavity 1 at (18, 0);
        cavity 6 at (15, 1);
        cavity 5 at (16, 1);
        cavity 4 at (17, 1);
        cavity 3 at (18, 1);
    }
}
```

`layout face` coordinates are slot coordinates on the connector face. A normal
cavity occupies one slot; `size large` occupies a 2x2 slot cavity. The renderer
normalizes the authored coordinates to the occupied cavities, so leading empty
slots are not shown as extra padding.

## Inline connectors

An `inline` member is a mated plug/receptacle pair mid-harness — the branch
point where a big loom splits off a smaller one (an engine-bay loom feeding a
pedal stub, say). It is **one shared port set with two housing halves**: mated
pin N is electrically pin N on either side, and each half references the
`connector_type` of its physical housing.

```
inline ic_pedal "Pedal branch" {
    male:   Dt04_4p;      // connector_type refs, imported via `use` as usual
    female: Dt06_4s;
    port apps1 "APPS1" pin 1;
    port apps2 "APPS2" pin 2;
    port gnd   "GND"   pin 3;
}
```

Format and semantics:

- `inline <name> ["<label>"] { <half>* <port>* }` — a component member,
  alongside instances and wires. It belongs in the loom fragment that owns the
  branch point.
- `male:`/`female:` each name a `connector_type` (where `part:` and future
  terminal/seal part numbers live). Both are optional and each may appear at
  most once; halves and ports may interleave.
- Ports use the ordinary connector-body port form. **No `pub`** — inline
  ports are always addressable within the defining (merged) component, so
  `pub` is meaningless and warns.
- Inlines share the **instance namespace**: wire endpoints address them like
  an instance (`[vcm.accel_1, ic_pedal.apps1]`), and a name collision with an
  instance is a duplicate error. Both looms' conductors land on the *same*
  ports — the engine-side cable ends at `ic_pedal.apps1` and the stub-side
  cable starts there; that shared endpoint is the electrical continuity
  through the mated pair.

**Harness views select a housing half:**

```
view harness "Cab loom — pedal branch" {
    include vcm.accel       at (0, 0);
    include ic_pedal.female at (36, 0);   // the half crimped onto THIS loom
}

view harness "Pedal stub" {
    include ic_pedal.male at (0, 0);
    include pedal.x1      at (36, 0);
}
```

Each loom's drawing includes the half physically built onto that bundle; the
node's header shows that half's description and `part` plus an **M/F chip**,
and auto-scoping keeps the other loom's conductors out of the drawing. The
half must be declared on the inline; including **both halves in one view is
an error** (every conductor would attach to both nodes), and an inline cannot
appear in a schematic include. Pinout views of the halves are a future
refinement.

## Instantiation

A *type* is a definition; an *instance* is a placement of that definition. To instantiate, write the instance name first, then a colon and the type — like a Rust `let` binding or struct field:

```
<instance_name>: <TypeName> "<optional label>";
```

Examples:

```
component Vehicle {
    pack:    BatteryPack "HV Battery";
    inv:     Inverter    "Motor Controller";
    m:       Motor       "Drive Motor";
    charger: Obc         "On-Board Charger";
    conv:    Dcdc        "DC-DC Converter";
}
```

The instance name (`pack`, `inv`) is what wires reference. The label (`"HV Battery"`, `"Motor Controller"`) is what diagrams display.

## Wires

```
wire <color>[/<tracer>] <gauge_mm2> ["<label>"] [<endpoint>, <endpoint>, ...];
wire <color>[/<tracer>] <gauge_mm2> ["<label>"] [<endpoint>, <endpoint>, ...] { <properties> }
```

- `color` — bare identifier from the IEC 60757 vocabulary: `black`, `brown`, `red`, `orange`, `yellow`, `green`, `blue`, `violet`, `grey`, `white`, `pink`, `turquoise`, `gold`, `silver` (synonyms `purple` and `gray` normalise to `violet`/`grey`). Any other name still renders verbatim but raises a `check` warning (fatal under `--strict`). A two-tone (tracer/striped) wire writes base and tracer separated by a slash — `green/yellow` — and draws as the base colour with a dashed tracer overlay; colour-code annotations join the IEC 60757 codes with a slash (`GN/YE`).
- `gauge_mm2` — number in mm² (e.g., `50`, `4`, `0.5`, `0.25`). It must be positive: `check` rejects `0` or a negative gauge (`invalid_gauge`).
- `"<label>"` — optional signal name (e.g., `"HV+"`, `"CAN H"`), shown on the wire in a harness drawing. Omit it when the net name adds nothing.
- The bracketed list contains two or more endpoints. Each endpoint is `instance.port`, or a bare name: the enclosing component's own port (a `pub` port being wired to internals), or one of its splices (see Splices). An endpoint may appear only once per wire: `check` rejects `[ecu.gnd, ecu.gnd]` (`duplicate_endpoint`). Two separate wires ending on the same port are fine — that is a double crimp.

```
// two-endpoint wire
wire orange 50 [pack.hv_pos, inv.dc_pos];

// labelled wire (the label shows on harness cables)
wire orange 50 "HV+" [pack.hv_pos, inv.dc_pos];

// multi-endpoint wire (T-junction / shared rail)
wire orange 50 [pack.hv_pos, inv.dc_pos, charger.hv_pos, conv.hv_pos];

// reference parent component's own pub port (no instance prefix)
wire orange 50 [hv_pos, c_main_pos.out];
```

Multi-endpoint wires model shared rails (HV power bus, enable signals to parallel coils): where the branches meet is left to the build. When the joint is a physical part of the harness — a crimp or solder splice in the loom, with conductors of different gauges or colors — declare it as a `splice` and run one wire to it from each pin instead (see Splices).

### Wire properties

A wire may end in a `{ }` body of `key: value;` lines instead of its `;` — the same shape as a cable's properties, and like any brace block it takes **no** `;` after the `}`. The body carries the physical data a wire list or BOM needs:

```
wire red 0.5 "12V" [fuses.ecu, ecu.b_pos] {
    length: 1.2;
    type:   "FLRY-B";
    part:   "FLRY-B-0.5-RD";
}

wire black 1.5 "GND" [ecu.gnd, ecu.case_gnd] { length: 150mm; }
```

- `length: <length>;` — the wire's length (see Lengths below). It must be positive (`invalid_length`). On a wire with more than two endpoints `length` is the length of the whole rail, not of each leg; for per-leg lengths use a splice with one wire per leg (see Splices).
- `type: "<string>";` — the insulation/construction type, free text (e.g. `"FLRY-B"`).
- `part: "<string>";` — the part number.

All three are optional, and each may appear once. Any other key is an error (`unknown_cable_property`, the code cable, wire and component bodies share), as is a key set twice (`duplicate_cable_property`) or a value of the wrong kind — a string for `length`, a number for `type` or `part` (`cable_property_type`). The positional form without a body is unchanged.

### Lengths

A length — a wire's or a cable's `length` — is a positive number with an optional unit written **directly after it**, no space between:

- `length: 2;` — a bare number is **metres**.
- `length: 2m;`, `length: 15cm;`, `length: 150mm;` — the units are exactly `m`, `cm` and `mm` (no inches, no `km`). Pick whichever reads best: a 150 mm jumper is clearer as `150mm` than as `0.15`.

Every length normalises to metres (to the micrometre), so `150mm`, `15cm` and `0.15` are the same length, and a harness cable box shows it in metres (`0.15 m`). Any other suffix is an error on the unit (`unknown_length_unit`, its help listing the units); a length that is zero, negative or under a micrometre is `invalid_length`, labelled on the number and its unit. `length: 2 m;` (a space before the unit) is a syntax error that says to write the unit directly after the number.

**Only a length takes a unit.** Every other number is bare — a wire's gauge (mm², `wire red 1.5 …`, not `1.5mm`), a view's `grid:`, `at (x, y)` coordinates, pin numbers, a layout's `rows`/`cols`, a `connector_type` property: a unit on any of them is a syntax error. Since a unit is whatever touches the digits, a number directly followed by a name is one token: write `cavity 1 at (…)`, never `cavity 1at (…)`.

## Cables

A `cable` groups point-to-point wires that travel together as one physical bundle (a twisted pair, a shielded multi-core, a motor-phase loom) and carries its construction metadata and its screen (see Shields). It's the WireViz "cable": connector pin tables on the sides, a labelled cable box in the middle.

```
cable motor_phases "Motor phases" {
    type:   "Shielded 3-core";
    length: 1.2;

    wire orange 35 "U" [inv.phase_u, mot.phase_u];
    wire orange 35 "V" [inv.phase_v, mot.phase_v];
    wire orange 35 "W" [inv.phase_w, mot.phase_w];
}
```

Format:

- `cable <name> ["<label>"] { <properties> <wires> [<shield>] }` — `name` is a snake_case designator (unique within the component, like a connector's); the optional quoted label is shown on the cable box (the designator is used when omitted).
- Properties are `key: value;` lines, before the wires. Two keys are supported:
  - `type: "<string>";` — a free-text construction note (e.g. `"Twisted pair"`).
  - `length: <length>;` — the cable's length: metres when bare (`2.5`), or with a unit (`2.5m`, `80cm`, `800mm`; see Lengths). It must be positive: `check` rejects `0` or a negative length (`invalid_length`).
- A cable needs at least one conductor: `check` warns on a cable with no wires (`empty_cable`).
- Each `wire` uses the exact same syntax as a loose wire, **but must have exactly two endpoints** — a cable conductor is one physical run from one pin to another. (A shared rail that fans out to three+ pins is not a single conductor; keep it a loose multi-endpoint `wire`, outside any cable.)
- A conductor may carry a property body like any wire (see Wire properties), inside a `twisted { }` group too. A conductor's own `length` **overrides** its cable's `length` for that conductor — a phase lead that routes further than its siblings, say; the conductors without one take the cable's. The cable's own `length` stays what the cable box shows. A cable body itself takes only `type` and `length` (`part` is a wire key).
- A `twisted { <wire> <wire> }` group wraps exactly two conductors that are twisted together as a pair. Groups and plain wires may interleave freely, so one cable can carry straight power conductors alongside a twisted signal pair:

```
cable sensor_loom "Sensor loom" {
    length: 2m;

    wire red 1.5 "12V" [ecu.pwr, sensor.pwr] {
        length: 230cm;              // routes round the bracket: longer than the loom
    }
    twisted {
        wire white/blue 0.5 "SIG H" [ecu.h, sensor.h];
        wire white/red 0.5 "SIG L" [ecu.l, sensor.l];
    }
    wire black 1.5 "GND" [ecu.gnd, sensor.gnd];
}
```

A `twisted { }` group holds **exactly two** conductors — a twisted pair — and that count is grammar-enforced: any other number of wires inside the braces is a parse error. In a harness drawing the pair braids inside the cable box.

### Shields

A screened cable, or an individually screened twisted pair, declares its screen with a `shield` line — in the cable body for the cable's overall screen, inside the `twisted { }` group for the pair's own:

```
cable can2 "CAN2" {
    type:   "Shielded twisted pair";
    length: 1.6;

    twisted {
        wire white/blue 0.5 "CAN H" [rear.can_h, vcm.can_h];
        wire white/red  0.5 "CAN L" [rear.can_l, vcm.can_l];
        shield [vcm.shield_gnd];             // the pair's own screen
    }
    shield "SHLD" [vcm.shield_gnd];          // the cable's overall screen
}
```

```
shield ["<label>"] [<endpoint>, <endpoint>];   // grounded at both ends
shield ["<label>"] [<endpoint>];               // grounded at one end (the usual)
shield ["<label>"];                            // lands on no pin: cut back, or bonded at a gland or housing
```

- A shield is **not a conductor**: no color, no gauge, and it does not count toward a pair's two wires or a cable's conductors (a cable holding only a shield still warns `empty_cable`).
- The bracketed list names the pins its **drain** is terminated on, written like wire endpoints (`instance.port` or an own `port`) and resolved the same way (unknown or private ports are errors). Zero, one or two ends: a screen runs from one end of the cable to the other, so more than two is an error (`shield_arity`). An empty `[]` is a syntax error — write `shield;`, which says only that the screen lands on no pin: it may be cut back (floating) or bonded all round at a cable gland or housing, and the model does not record which (say so in a comment).
- **At most one shield** in a cable body and at most one in each `twisted { }` group; a second is an error naming both (`duplicate_shield`). A cable's overall screen and its pairs' own screens are separate, so both may be present.
- An end listed twice is `duplicate_endpoint`. A drain sharing a pin with a wire (a drain crimped alongside a ground wire) is fine.
- The optional label names the drain in a harness drawing; the drawing calls an unlabelled one `Shield` (or `Pair shield`).
- In a harness view a shielded cable's box gets a dashed inner outline and `shielded` in its subtitle; a screened pair gets a dashed outline round its rows. Each shield end on a connector the view includes is drawn as a thin dashed drain line from the cable box to that pin (the pin gets a row in its table); an end on a connector the view leaves out is not drawn. A shield never puts a cable box in a view by itself: its cable is drawn only when its conductors are.
- `shield` is a keyword only at the start of a statement in a cable body or a `twisted { }` group; anywhere else it is an ordinary name (`pub port shield "SHIELD";`, `[ecu.shield]`).

A cable's wires are still ordinary connections: they show in schematic views like any other wire. The cable grouping adds the harness box and the BOM metadata.

**Cables vs. shared rails.** Use a `cable` for a real bundle of two-pin conductors. Use a loose multi-endpoint `wire` for a bus/rail that branches to many pins, or a `splice` when the branch point is a joint built into the harness (see Splices); a cable conductor may end on a splice.

## Splices

A `splice` is a joint inside a component — a crimp or solder splice in a loom — that conductors end on. It gives the joint an identity (a name, a label, a part), and each conductor to it is an ordinary two-ended wire with its own color, gauge and length:

```
component Vehicle {
    ecu:  Ecu  "ECU";
    lamp: Lamp "Lamp";
    bat:  Battery "Battery";

    splice sp_gnd "Ground splice" {
        part:        "D-436-36";
        description: "Solder sleeve";
    }

    wire black 1   "GND" [ecu.gnd,  sp_gnd];
    wire black 0.5 "GND" [lamp.gnd, sp_gnd];
    wire black 2.5 "GND" [sp_gnd, bat.neg];
}
```

```
splice <name> ["<label>"];
splice <name> ["<label>"] { part: "<string>"; description: "<string>"; }
```

- A splice is a member of a component (or `extend`) body. The optional label names it in a drawing; the optional body takes `part:` (the splice's part number) and `description:`, both strings, each once — any other key is `unknown_cable_property`, a repeat `duplicate_cable_property`, a number `cable_property_type`. Like a wire's body, it ends at its `}` with no `;`.
- A conductor ends on a splice by its **bare name**, where an endpoint goes: `[ecu.gnd, sp_gnd]`. A bare name is either an own port or a splice, so a splice's name must differ from every port, instance, inline connector, connector designator, cable designator and other splice of the component (`duplicate_splice_name`). In a component split with `extend`, a splice is visible to every fragment, like a port.
- **All conductors ending on one splice are joined.** Any wire may end on one: a loose wire, a cable conductor (a cable's conductor can run from a pin to a splice), or a wire from one splice to another (the two joints are then one net).
- A wire to a splice has **exactly two ends** (`splice_wire_arity` otherwise): a splice joins point-to-point conductors. A shared rail without a splice stays a loose multi-endpoint wire, valid as ever; the two are for different things.
- A splice that fewer than two conductors end on joins nothing (`splice_joins_nothing`, a warning, fatal under `--strict`). Two is fine: a butt splice, for a gauge or color change.
- A shield's drain lands on a pin, never on a splice (`shield_on_splice`).
- A bare endpoint that is neither a port nor a splice is `unknown_port`, its help saying a bare name may be either.
- `splice` is a keyword only at the start of a statement in a component body, followed by a name; anywhere else it is an ordinary name (`pub port splice "S";`, an instance `splice: Sensor;`, an endpoint `ecu.splice`).

## Views

A view declares a rendering target — what to render, at what positions, with what grid scale. There are three kinds: **`schematic`** (component boxes with ports on authored sides), **`harness`** (WireViz-style connector pin tables with cable bundles), and **`pinout`** (physical connector faces plus pin tables). They differ in what an `include` selects.

Every view documents one component, its **subject**: includes, `ports { }` placements and `enclosure { }` ports all resolve against it. By default the subject is the file's single top-level component. Name it explicitly with `for` after the title:

```
component Sensor { connector x1 "Sensor 2p" { pub port sig "SIG" pin 1; } }
component Lamp   { connector x2 "Lamp 2p"   { pub port feed "FEED" pin 1; } }

view pinout "Sensor pinout" for Sensor {
    include x1 at (0, 0);
}

view pinout "Lamp pinout" for Lamp {
    include x2 at (0, 0);
}
```

- `for <Component>` names a component the file defines at its top level or imports with `use` (a view naming an import counts as using it). In an `extend` fragment it may name the extended component. A nested component cannot be a subject.
- Without `for`, a file with no or several top-level components cannot tell which one a view documents (`view_subject`, an error); with `for`, one file can hold views of several components. Naming the file's only component is allowed, just optional.
- A name that is not in scope is `undefined_type`; a `connector_type` is `view_subject_not_component`.

The view is drawn against an instance of its subject: a view of a component the design never instantiates (say, in a library file used only for its connector types) is the `uninstantiated_view_subject` error.

### Schematic views

Each `schematic` include also says which of that component's ports to show, on which side, and in what order.

```
view schematic "System Overview" {
    grid: 20;

    include pack at (2, 5) ports {
        east: hv_pos, hv_neg, can_h, can_l;
    }

    include inv at (15, 2) ports {
        west:  dc_pos, dc_neg, can_h, can_l;
        east:  phase_u, phase_v, phase_w;
        south: enable;
    }

    include conv at (15, 18);   // bare box: no ports shown
}
```

Format:

- `view <kind> "<title>" [for <Component>] { ... }` — `kind` is `schematic`, `harness`, or `pinout`; `for` names the subject (see above). Titles are unique across the project: a repeated title (in any file) is the `duplicate_view_title` warning; the repeat still renders, to a `_2`-suffixed file.
- `grid: <n>;` — pixels per grid cell. Optional; a sensible default applies. It must be positive (`invalid_grid`), and in a `schematic` at least 7.5: ports sit two steps apart and the pitch must clear a port label, so `check` rejects a finer grid (`grid_too_fine`, naming the minimum). Harness and pinout views take any positive grid.
- `include <name> at (x, y);` or `include <name> at (x, y) ports { ... }` — place a component by its instance name in the surrounding scope, at grid-cell coordinates (negative is fine). A bare include is a statement and ends with `;`; an include with a `ports { }` block ends at the closing brace, with **no** trailing `;` (like any other brace block).
- `text "<label>" at (x, y);` — a free-text note at grid-cell coordinates (see Text boxes).
- `ports { <side>: <port>, <port>; ... }` — optional. Each line lists the ports on one `side` (`north`, `east`, `south`, `west`), in the order they should appear on that edge. A `west: a, b;` line puts `a` above `b` on the west edge. List the same side more than once and the lines concatenate.

**The `ports` block controls both layout and scope.** A port is drawn only if it's listed; everything else is hidden. An include with no `ports` block is a bare labelled box.

**Wires are NOT listed in views.** A wire from the model is drawn only when *both* of a segment's endpoints are listed ports on included components; otherwise that segment drops silently. So a listed port whose wire goes to an unlisted (or excluded) port shows as a bare stub with no line. A splice never appears in a schematic: the ports its conductors join are drawn connected, like a multi-endpoint rail between the ones the view lists (through splice-to-splice links too).

Sides are authored, never guessed: place a port on the side facing the box it wires to, and line the two boxes up so the wire runs straight (ports sit two grid steps apart, centred on each side). Private (non-`pub`) ports cannot be placed in a view, the same as they cannot be wired from outside.

Each port shows its pin number(s) outside the box (`2`, or `2,3` for a ganged port). On a component with more than one connector — counting all of them, not only those the view shows — the number is qualified with the port's connector designator, `hv:1` beside `intlk:1`, so a bare number always means the component's only connector (a port with a pin but no connector, the `bare_port_pin` warning, keeps the bare number). Short designators keep the drawing tidy: a pin text longer than the gap to the next port on a north or south side turns to run along its wire, and a long one reaches out across nearby wires.

Leave room between boxes. `check` lays each view out with the renderer's own box sizes and rejects includes whose boxes overlap (`overlapping_includes`, in every view kind; touching edges are fine). In a schematic a wire leaves its port by one grid step and keeps one step clear of every other box, so a wired port needs **two grid steps** of space before the box it faces (`include_too_close`), and boxes closer than two steps to each other leave no gap to route through (`unroutable_wire` when a wire is walled in). The enclosure is never in the way.

Keep a view compact: its drawing may span at most 8192 units on a side, margins, title and stamp included (`view_too_large`, an error in every view kind, naming the size and labelled on the title and the includes farthest apart). Positions count grid steps, so `at (1000, 0)` on `grid: 20;` is already 20000 units out; split a sprawling drawing into several views instead.

### Text boxes

A schematic view may carry free-text notes — a termination resistor, a fitting instruction — each drawn as a small boxed label:

```
view schematic "CAN bus" {
    grid: 20;

    include front at (8, 12) ports {
        south: can_h, can_l;
    }

    text "120 Ω termination" at (8, 4);
}
```

- `text "<label>" at (x, y);` — the label first, as every declaration writes its display string, then the box's **centre** in grid cells (like an include's `at`; negative is fine). A text box has no name: nothing references it. The box sizes itself to the label.
- Schematic views only: a `text` in a harness or pinout view is `unsupported_view_text`.
- A text box is an annotation, not an include: `check` does not test it for overlap, so keep it clear of the boxes and wires yourself.
- **Deprecated:** the older named form, `text <name> at (x, y) "<label>";`, still renders but will be removed. `check` warns on it (`deprecated_text_syntax`, fatal under `--strict`), its help showing the statement rewritten in the form above; two named boxes sharing a name in one view are `duplicate_view_text`. Write new text boxes label-first.

### Harness views

A `harness` include selects a whole **connector** by its designator and draws it as a pin table. Cables between connectors are derived from the model's wires.

```
view harness "Main HV harness" {
    grid: 20;

    include front.hv   at (4, 8);     // <instance>.<connector designator>
    include rear.hv     at (4, 22);
    include inv.hv      at (34, 8);
    include charger.hv  at (40, 22);
}
```

- `include <instance>.<connector> at (x, y);` — the target is `instance.connector` (the connector's designator on that instance's type). The pin table is **auto-scoped**: it draws only the pins that carry a conductor in this view, in pin order (an include none of whose pins are wired draws as a header-only box). **No `ports { }` block** — scope is derived from the view's cables, not listed — and **no side/facing** — pin facing is auto-oriented from where the cables go.
- A connector must have a designator to be included (see Connector blocks). Including the same instance's other connectors is just more `include` lines.
- `include <splice> at (x, y);` — places one of the subject's splices (see Splices), by its bare name: no `.connector`, no `ports { }`. It draws as a solid dot at `(x, y)` with the splice's name, label and part beneath it, and its conductors meet at the dot. A splice takes part in the overlap check like any include. A splice is placed in harness views only: in a schematic or pinout view the include is `wrong_include_form`.

**Cables are derived, like schematic wires.** A wire renders as a cable strand only when *both* of its endpoints land on *included* connectors or splices; ends on connectorless ports, excluded connectors or splices, or the parent's own ports drop silently. A conductor to a splice draws like any other: a cable's through its cable box, a loose wire as a loose wire. So connectorize the external `pub port`s you want to see in a harness (wrap them in a named `connector` block).

**Every conductor belongs in some harness view.** Once a component has at least one harness view, `check` warns about each wire segment between connectors of two of its children, and each conductor to one of its splices, that none of its harness views draws (`undrawn_conductor`, fatal under `--strict`) — the conductor would be missing from every build drawing. A multi-endpoint rail is judged per chain segment (`[a, b, c]` → `a–b`, `b–c`); a conductor to a splice is drawn when a view includes both its ends (the connector and the splice, or both splices); an inline connector is covered when a view includes either half; segments with an end on the parent's own port or a connectorless port, jumpers within one connector, and shields are not harness conductors. A component with no harness view is never reported. Fix it by adding the missing connectors (or splices) to a loom's view, or a view for the missing loom.

A wire that belongs to a declared `cable` (see Cables) draws WireViz-style: a labelled cable box sits between the two connectors it spans, titled with the cable's label and its `type · length`, one coloured strand per conductor. Loose wires between the same connector pair instead bundle into a plain strand group, each strand drawn in its `color` and annotated with its `label` and gauge.

### Pinout views

A `pinout` include selects a connector owned by the view subject itself. This
is useful for connector reference drawings when building a harness.

```
view pinout "Inverter pinouts" {
    grid: 20;

    include hv at (0, 0);
    include control at (14, 22);
}
```

- `include <connector> at (x, y);` — the target is a connector designator on
  the subject component, not a child instance. Use `include control`, not
  `include inv.control`.
- No `ports { }` block. The whole connector is drawn from the connector type's
  layout and pin bindings.
- The drawing uses the harness-side convention described above, and each
  cavity face says so in a `Viewed from harness side` caption.
- The table is headed by the component (its label when it is placed once,
  else its type, as the drawing serves every placement) and the
  designator, with the connector type's description and `part` beneath; its
  rows list the pins in numeric order, whatever order they are declared in,
  then any port the connector gives no pin (`no pin`).
  Cavities no port uses are drawn dashed and greyed; a cavity label too long
  for its cavity shrinks, then is cut with an ellipsis (the table keeps it
  whole), so keep labels short where cavities are small. The face sits in a
  panel of its own on top of the table; each is as wide as its own content.
- The table is a crimping sheet: each pin's row lists what is crimped into
  the cavity, from the design's wires — per conductor a colour swatch with
  its IEC code, the gauge and the far end (`→ Vehicle.vcm.wake (x1:3)`; a
  splice with what it joins, `→ Vehicle.sp_gnd (lamp.gnd, +2 more)`; a
  wire-through out of the component's own connectorless port,
  `→ Vehicle.loom.out (leaves Loom)`), a
  double crimp as two lines under the pin, a screen's drain as `drain`
  (no colour or gauge), an unwired pin as a dash. A ganged port's
  conductors appear on each of its pins, marked `ganged 46+48`. Wires
  belong to a placement: the table lists the first placement's, and when
  the type is placed more than once the header names it (`Wires shown for
  Vehicle.front.bms, one of 2 placements`).

Views live in the same file as the component they primarily document — like unit tests in Rust source. A view of the system overview belongs in `main.wb` where `Vehicle` is defined. A view of the battery pack detail belongs in the file where `BatteryPack` is defined. This keeps views close to the data they describe. In a file that defines several components, give each view a `for`.

## Naming conventions

- **Types are CamelCase; instances are snake_case** — Rust's convention. `CellModule` is a type (component or connector_type); `module_1` is an instance. The grammar accepts any identifier in either position (case is convention, not enforced), but follow the convention in all authored files.
- **Port names** are short, snake_case: `hv_pos`, `coil_p`, `iso_spi_up_h`. Polarity suffixes: `_p`/`_n` for signal pairs, `_pos`/`_neg` for power, `_h`/`_l` for differential pairs.
- **Port labels** are short, human-readable, quoted: `"HV+"`, `"COIL+"`, `"ISO SPI+"`.
- **Instance names** can carry role: `c_main_pos`, `c_precharge`, `module_1`, `inv`. Short is fine when context is clear.

### Reserved and contextual words

Keywords come in two kinds. Words are case-sensitive, so `Layout` or `Wire` is never a keyword.

- **Reserved** — the words that declare the design: `use`, `component`, `extend`, `connector_type`, `connector`, `inline`, `port`, `pub`, `pin`, `pins`, `wire`, `cable`, `twisted`, `view`. They can never name anything; `pub port wire "W";` is an error, `'wire' is a reserved word`. Pick another name (`wire_1`, `w`).
- **Contextual** — words that mean something only inside a view (or after its title), a `connector_type` layout, a cavity line, a `use` line, at the start of a top-level statement, at the start of a statement in a cable body or a `twisted { }` group, or at the start of a statement in a component body followed by a name: `from`, `fragment`, `shield`, `splice`, `for`, `include`, `text`, `at`, `grid`, `enclosure`, `ports`, `layout`, `face`, `cavity`, `rows`, `cols`, `numbering`, `size`. Anywhere a name belongs they are ordinary names, so `pub port size "SIZE";`, `pub port for "FOR";`, an instance `grid: Sensor;`, `connector layout: Plug { … }`, `cable text { … }` and `include at at (1, 2);` are all valid, and so are `pub port fragment "F";`, an instance `fragment: Lamp;`, `pub port shield "SHIELD";`, an endpoint `ecu.shield`, `pub port splice "SPLICE";` and an instance `splice: Sensor;`.

Other words the grammar reads in one place only (`schematic`, `harness`, `pinout`, `type`, `length`, `part`, `manufacturer`, `description`, `rating`, `male`, `female`, `north`, `east`, `south`, `west`, the wire colors, the numbering modes, `large`) are not keywords at all.

## Common patterns

**Series chain through a composite's external ports** — wires alternate between connecting external pub ports and chaining internal instances:

```
wire orange 50 [hv_pos,          c_main_pos.out];
wire orange 50 [c_main_pos.in,   module_1.hv_pos];
wire orange 50 [module_1.hv_neg, module_2.hv_pos];
// ...
wire orange 50 [module_n.hv_neg, c_main_neg.in];
wire orange 50 [c_main_neg.out,  hv_neg];
```

**Daisy chain between paired ports** — each instance has an "up" port and a "down" port; wire down-of-N to up-of-N+1, leaving the last one open:

```
wire black 0.25 [iso_spi_p,                module_1.iso_spi_up_h];
wire black 0.25 [module_1.iso_spi_down_p,  module_2.iso_spi_up_h];
wire black 0.25 [module_2.iso_spi_down_p,  module_3.iso_spi_up_h];
// module_3.iso_spi_down_* unconnected — end of chain
```

**Parallel coils via one multi-endpoint wire** — many coils driven by the same control signal:

```
wire black 1.0 [enable_p, c_main_pos.coil_p, c_main_neg.coil_p, c_precharge.coil_p];
wire black 1.0 [enable_n, c_main_pos.coil_n, c_main_neg.coil_n, c_precharge.coil_n];
```

**Shared HV bus** — a single multi-endpoint wire connecting every HV consumer to the pack:

```
wire orange 50 [pack.hv_pos, inv.dc_pos, charger.hv_pos, conv.hv_pos];
wire orange 50 [pack.hv_neg, inv.dc_neg, charger.hv_neg, conv.hv_neg];
```

## Do not

- **Do not** include wires in views. They're derived from the model; you control which show by listing their ports.
- **Do not** expect a port to appear in a view unless you list it in that include's `ports` block. Listing is the scope.
- **Do not** rely on filesystem structure for hierarchy. Only `use`, `fragment` and nesting matter.
- **Do not** pull a file of `extend`s in with `use Vehicle from "x.wb";` in new code: write `fragment "x.wb";`. (The `use` spelling still works.)
- **Do not** reference unexposed (non-`pub`) ports of nested components from outside. Private is private.
- **Do not** introduce new keywords or option fields that aren't documented in this skill or already used in the project's `.wb` files. If something is awkward to express, raise it with the user rather than inventing syntax.
- **Do not** invent junction nodes: a rail is a multi-endpoint wire, and a physical joint is a `splice` with one two-ended wire to it per pin. Do not list a splice among more than two endpoints, or name one as a shield end.
- **Do not** put `pub` on an inline connector's ports, include both halves of one inline in a single harness view, or include an inline in a schematic view.
- **Do not** write a `;` after a wire's property body (`[a, b] { length: 1; }`, not `{ … };`), or put `part` on a cable — it is a per-wire key.
- **Do not** put a space between a length and its unit (`150mm`, not `150 mm`), or a unit on any number but a `length` (a gauge is `1.5`, not `1.5mm`).
- **Do not** put part numbers in labels or comments when the type can carry them: write `part: "…";` (and `manufacturer`, `description`, `rating`) in the component body. Quote the value — `part: Fuse;` is an instance named `part`, not a part number.
- **Do not** put a multi-endpoint wire inside a `cable` — cable conductors are point-to-point (exactly two endpoints). Keep fan-out rails as loose wires.
- **Do not** model a screen as a `wire` with an invented color: write `shield [<pin>];` in the cable body (or the pair's `twisted { }`), with at most two ends and one shield per cable body or pair.
- **Do not** name a text box (`text note at (0, 4) "…";` is the deprecated form): write the label first, `text "…" at (0, 4);`.
- **Do not** use block comments (`/* */`). Line comments (`//`) only.
- **Do not** mix grid coordinates with pixel coordinates. View positions are in grid cells.
