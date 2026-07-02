---
name: wirebug-dsl
description: Use this skill whenever editing, creating, reviewing, or discussing wirebug DSL files (.wb extension) — a text-defined electrical schematic and wiring harness language for the Aphid EV project. Covers component definitions, type/instance separation, ports (pub vs private), connector groupings, multi-endpoint wires, hierarchical composition, imports between files, views, and grid-based layout. Trigger whenever the user mentions wirebug, .wb files, components, schematics, harnesses, EV wiring, or is designing electrical systems in text. Trigger whenever a .wb file is open, being edited, or referenced.
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

Filesystem layout is for human navigation only. **A component's logical position in the hierarchy is determined entirely by `use` statements and nesting in the DSL**, not by what directory the file lives in. Moving a `.wb` file to a different folder must never change the model — the only thing that matters is which paths the `use` statements resolve to.

## File contents

A `.wb` file contains zero or more of, in any order:

- `use` declarations (imports from other files)
- `component` definitions (type definitions, which may nest other definitions)
- `view` declarations (rendering targets)

Comments are `//` line comments only. Block comments are not supported.

Statements end with `;`; brace blocks end at the closing brace with no trailing `;`. Comma-separated lists — `pins [ ]`, wire endpoint lists, and view `ports` side lists — allow a trailing comma, so multi-line lists diff cleanly. Numbers may be negative (`at (-2, 4)`); there is no other arithmetic.

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

## Splitting a component across files (`extend`)

A large top-level component — the vehicle root, say — can be authored across
several files. One file introduces it with `component`; others add to it with
`extend`. `main.wb` pulls the fragments in with ordinary `use` statements:

```
// main.wb
use Vehicle from "traction.wb";
use Vehicle from "charging.wb";
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
- A `use Vehicle from "traction.wb"` both loads the file and triggers the
  merge (a same-name collision with `extend` on either side merges instead of
  erroring). Without `extend`, two same-named `component`s are still a
  duplicate-type error.
- Each fragment carries its **own** `use` imports for the types it
  instantiates (`traction.wb` imports `Inverter` itself).
- The merged component is **one flat namespace**: a wire or view in any
  fragment may reference an instance or port declared in any other fragment
  (`traction.wb` wires to `pack`, declared in `main.wb`). Views always see the
  whole merged component.
- `extend` is top-level only (not nested), and every `extend <name>` needs a
  `component <name>` somewhere — a lone `extend` is an error.

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
```

- `color` — bare identifier from the IEC 60757 vocabulary: `black`, `brown`, `red`, `orange`, `yellow`, `green`, `blue`, `violet`, `grey`, `white`, `pink`, `turquoise`, `gold`, `silver` (synonyms `purple` and `gray` normalise to `violet`/`grey`). Any other name still renders verbatim but raises a `check` warning (fatal under `--strict`). A two-tone (tracer/striped) wire writes base and tracer separated by a slash — `green/yellow` — and draws as the base colour with a dashed tracer overlay; colour-code annotations join the IEC 60757 codes with a slash (`GN/YE`).
- `gauge_mm2` — number in mm² (e.g., `50`, `4`, `0.5`, `0.25`).
- `"<label>"` — optional signal name (e.g., `"HV+"`, `"CAN H"`), shown on the wire in a harness drawing. Omit it when the net name adds nothing.
- The bracketed list contains two or more endpoints. Each endpoint is `instance.port` or just `port` if referencing the enclosing component's own port (a `pub` port being wired to internals).

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

Multi-endpoint wires model shared rails (HV power bus, enable signals to parallel coils). The renderer derives junction dots from the topology — don't try to add junction nodes manually.

## Cables

A `cable` groups point-to-point wires that travel together as one physical bundle (a twisted pair, a shielded multi-core, a motor-phase loom) and carries its construction metadata. It's the WireViz "cable": connector pin tables on the sides, a labelled cable box in the middle.

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

- `cable <name> ["<label>"] { <properties> <wires> }` — `name` is a snake_case designator (unique within the component, like a connector's); the optional quoted label is shown on the cable box (the designator is used when omitted).
- Properties are `key: value;` lines, before the wires. Two keys are supported:
  - `type: "<string>";` — a free-text construction note (e.g. `"Twisted pair"`).
  - `length: <number>;` — length in **metres** (a bare number, e.g. `2.5`; no unit suffix).
- Each `wire` uses the exact same syntax as a loose wire, **but must have exactly two endpoints** — a cable conductor is one physical run from one pin to another. (A shared rail that fans out to three+ pins is not a single conductor; keep it a loose multi-endpoint `wire`, outside any cable.)
- A `twisted { <wire> <wire> }` group wraps exactly two conductors that are twisted together as a pair. Groups and plain wires may interleave freely, so one cable can carry straight power conductors alongside a twisted signal pair:

```
cable sensor_loom "Sensor loom" {
    length: 2.0;

    wire red 1.5 "12V" [ecu.pwr, sensor.pwr];
    twisted {
        wire white/blue 0.5 "SIG H" [ecu.h, sensor.h];
        wire white/red 0.5 "SIG L" [ecu.l, sensor.l];
    }
    wire black 1.5 "GND" [ecu.gnd, sensor.gnd];
}
```

A `twisted { }` group holds **exactly two** conductors — a twisted pair — and that count is grammar-enforced: any other number of wires inside the braces is a parse error. In a harness drawing the pair braids inside the cable box.

A cable's wires are still ordinary connections: they show in schematic views like any other wire. The cable grouping adds the harness box and the BOM metadata.

**Cables vs. shared rails.** Use a `cable` for a real bundle of two-pin conductors. Use a loose multi-endpoint `wire` for a bus/rail that branches to many pins. Explicit junction elements are not in the language yet.

## Views

A view declares a rendering target — what to render, at what positions, with what grid scale. There are three kinds: **`schematic`** (component boxes with ports on authored sides), **`harness`** (WireViz-style connector pin tables with cable bundles), and **`pinout`** (physical connector faces plus pin tables). They differ in what an `include` selects.

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

- `view <kind> "<title>" { ... }` — `kind` is `schematic`, `harness`, or `pinout`.
- `grid: <n>;` — pixels per grid cell. Optional; a sensible default applies.
- `include <name> at (x, y);` or `include <name> at (x, y) ports { ... }` — place a component by its instance name in the surrounding scope, at grid-cell coordinates (negative is fine). A bare include is a statement and ends with `;`; an include with a `ports { }` block ends at the closing brace, with **no** trailing `;` (like any other brace block).
- `ports { <side>: <port>, <port>; ... }` — optional. Each line lists the ports on one `side` (`north`, `east`, `south`, `west`), in the order they should appear on that edge. A `west: a, b;` line puts `a` above `b` on the west edge. List the same side more than once and the lines concatenate.

**The `ports` block controls both layout and scope.** A port is drawn only if it's listed; everything else is hidden. An include with no `ports` block is a bare labelled box.

**Wires are NOT listed in views.** A wire from the model is drawn only when *both* of a segment's endpoints are listed ports on included components; otherwise that segment drops silently. So a listed port whose wire goes to an unlisted (or excluded) port shows as a bare stub with no line.

Sides are authored, never guessed: place a port on the side facing the box it wires to, and line the two boxes up so the wire runs straight (ports sit two grid steps apart, centred on each side). Private (non-`pub`) ports cannot be placed in a view, the same as they cannot be wired from outside.

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

**Cables are derived, like schematic wires.** A wire renders as a cable strand only when *both* of its endpoints land on *included* connectors; ends on connectorless ports, excluded connectors, or the parent's own ports drop silently. So connectorize the external `pub port`s you want to see in a harness (wrap them in a named `connector` block).

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
- The drawing uses the harness-side convention described above.

Views live in the same file as the component they primarily document — like unit tests in Rust source. A view of the system overview belongs in `main.wb` where `Vehicle` is defined. A view of the battery pack detail belongs in the file where `BatteryPack` is defined. This keeps views close to the data they describe.

## Naming conventions

- **Types are CamelCase; instances are snake_case** — Rust's convention. `CellModule` is a type (component or connector_type); `module_1` is an instance. The grammar accepts any identifier in either position (case is convention, not enforced), but follow the convention in all authored files.
- **Port names** are short, snake_case: `hv_pos`, `coil_p`, `iso_spi_up_h`. Polarity suffixes: `_p`/`_n` for signal pairs, `_pos`/`_neg` for power, `_h`/`_l` for differential pairs.
- **Port labels** are short, human-readable, quoted: `"HV+"`, `"COIL+"`, `"ISO SPI+"`.
- **Instance names** can carry role: `c_main_pos`, `c_precharge`, `module_1`, `inv`. Short is fine when context is clear.

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
- **Do not** rely on filesystem structure for hierarchy. Only `use` and nesting matter.
- **Do not** reference unexposed (non-`pub`) ports of nested components from outside. Private is private.
- **Do not** introduce new keywords or option fields that aren't documented in this skill or already used in the project's `.wb` files. If something is awkward to express, raise it with the user rather than inventing syntax.
- **Do not** add junction nodes by hand — multi-endpoint wires imply junctions automatically.
- **Do not** put a multi-endpoint wire inside a `cable` — cable conductors are point-to-point (exactly two endpoints). Keep fan-out rails as loose wires.
- **Do not** use block comments (`/* */`). Line comments (`//`) only.
- **Do not** mix grid coordinates with pixel coordinates. View positions are in grid cells.
