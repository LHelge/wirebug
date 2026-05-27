---
name: wirebug-dsl
description: Use this skill whenever editing, creating, reviewing, or discussing wirebug DSL files (.wb extension) — a text-defined electrical schematic and wiring harness language for the Aphid EV project. Covers component definitions, type/instance separation, ports (pub vs private), connector groupings, multi-endpoint wires, hierarchical composition, imports between files, views, and grid-based layout. Trigger whenever the user mentions wirebug, .wb files, components, schematics, harnesses, EV wiring, or is designing electrical systems in text. Trigger whenever a .wb file is open, being edited, or referenced.
---

# wirebug DSL

wirebug is a text-defined DSL for describing electrical systems — components, wires, and the views that render them. Source files end in `.wb`. The DSL was designed for the Aphid EV conversion project but the syntax is general-purpose.

**This skill exists because the wirebug parser is not yet implemented.** The DSL is being designed by writing realistic models first, then implementing the parser to match. Treat the syntax here as authoritative for editing tasks; treat parseability as a future concern. Avoid inventing syntax that isn't documented here or already present in the project's existing `.wb` files — when in doubt, ask.

## Project structure

A wirebug project is a directory rooted at a file named `main.wb`. The presence of `main.wb` identifies a directory as a wirebug project (Cargo-style — its existence is the project marker; no separate manifest is required).

A conventional layout, which is **convention, not requirement**:

```
my-ev/
├── main.wb              # top-level vehicle component + system view
└── components/          # imported components, one type per file
    ├── battery_pack.wb
    ├── cell_module.wb
    ├── contactor.wb
    ├── inverter.wb
    └── ...
```

Filesystem layout is for human navigation only. **A component's logical position in the hierarchy is determined entirely by `use` statements and nesting in the DSL**, not by what directory the file lives in. Moving a `.wb` file to a different folder must never change the model — the only thing that matters is which paths the `use` statements resolve to.

## File contents

A `.wb` file contains zero or more of, in any order:

- `use` declarations (imports from other files)
- `component` definitions (type definitions, which may nest other definitions)
- `view` declarations (rendering targets)

Comments are `//` line comments only. Block comments are not supported.

## Imports

```
use cell_module from "cell_module.wb"
use contactor   from "components/contactor.wb"
```

Paths are relative to the importing file. A `use` brings one named top-level component definition into the current file's scope. Only top-level definitions can be imported — definitions nested inside other components are private to their parent.

If multiple imports come from the same file, repeat the statement (no list form yet):

```
use cell_module from "components/cell_module.wb"
use contactor   from "components/contactor.wb"
```

## Component definitions

A component is a **type**, not an instance. It defines an interface (its `pub` ports) and an implementation (instances of other types, plus wires connecting them).

A minimal primitive component (no internals, just an interface):

```
component contactor {
    pub port in     "IN";
    pub port out    "OUT";
    pub port coil_p "COIL+";
    pub port coil_n "COIL-";
}
```

Components can nest. A definition inside another component is scoped to its parent and is not exported — it can only be instantiated by code inside the containing definition:

```
component cell_module {
    component cell_pack {       // private to cell_module
        pub port hv_pos "+";
        pub port hv_neg "-";
        // ...
    }

    cell_pack pack;             // instantiate the nested type

    pub port hv_pos "HV+";      // external interface of cell_module
    pub port hv_neg "HV-";

    wire orange 50 [hv_pos, pack.hv_pos];   // external port wired to internal
    wire orange 50 [hv_neg, pack.hv_neg];
}
```

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
port name "Label" pins (N, N, N);   // multiple pins (e.g. ground or high-current)
```

Pin numbers are positive integers. A port may span multiple pins when several connector pins are ganged for current capacity (common for power and ground). Pin assignment is optional — omit it when the physical pin-out is unknown or irrelevant.

Example:

```
connector "Molex MX-150 12p" {
    pub port b_pos "B+" pin 1;
    pub port gnd   "GND" pins (2, 3, 4);
    pub port can_h "CAN H" pin 5;
    pub port can_l "CAN L" pin 6;
}
```

**Visibility does not propagate automatically.** If `cell_pack` declares `pub port hv_pos`, that port is visible when `cell_pack` is instantiated — but the containing `cell_module` does *not* automatically expose it. To re-export, the parent declares its own `pub port` and wires it through:

```
component cell_module {
    cell_pack pack;
    pub port hv_pos "HV+";                  // explicit external port
    wire orange 50 [hv_pos, pack.hv_pos];   // wired to inner port
}
```

This is more verbose than implicit propagation but does useful work: documents the abstraction boundary, allows renaming across boundaries (inner `hv_pos`, outer `hv_p`), prevents accidental leaks.

## Connector blocks

A `connector` block groups ports that physically belong to a single connector part (a JST XH 15p, a Deutsch DT06-12S, etc.). The block carries an **optional designator** (a snake_case reference name) and the part description as a string, and contains port declarations with optional pin assignments.

```
component cell_monitor {
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

The designator is **optional** but required to address the connector in a harness view (`include <inst>.<designator>`, see Views). Designators must be unique within a component.

Connectors are **structural metadata, not a namespace**. A port `c0` inside a connector is still referenced as `cell_monitor_instance.c0`, not `cell_monitor_instance.cells.c0` — the designator names the *connector*, not a port scope. The `connector` block carries physical-grouping info (including pin assignments) for the harness renderer and BOM. `pub` is independent — a connector can mix `pub` and non-`pub` ports freely.

## Instantiation

A *type* is a definition; an *instance* is a placement of that definition. To instantiate:

```
<TypeName> <instance_name> "<optional label>";
```

Examples:

```
component vehicle {
    battery_pack pack    "HV Battery";
    inverter     inv     "Motor Controller";
    motor        m       "Drive Motor";
    obc          charger "On-Board Charger";
    dcdc         conv    "DC-DC Converter";
}
```

The instance name (`pack`, `inv`) is what wires reference. The label (`"HV Battery"`, `"Motor Controller"`) is what diagrams display.

## Wires

```
wire <color> <gauge_mm2> ["<label>"] [<endpoint>, <endpoint>, ...];
```

- `color` — bare identifier (e.g., `orange`, `black`, `red`, `yellow`). Use CSS colour names; the harness renderer draws each strand in this colour.
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

A cable's wires are still ordinary connections: they show in schematic views like any other wire. The cable grouping adds the harness box and the BOM metadata.

**Cables vs. shared rails.** Use a `cable` for a real bundle of two-pin conductors. Use a loose multi-endpoint `wire` for a bus/rail that branches to many pins. Explicit junction elements are not in the language yet.

## Views

A view declares a rendering target — what to render, at what positions, with what grid scale. There are two kinds: **`schematic`** (component boxes with ports on authored sides) and **`harness`** (WireViz-style connector pin tables with cable bundles). Both document the file's single top-level component and render its direct children; they differ in what an `include` selects.

### Schematic views

Each `schematic` include also says which of that component's ports to show, on which side, and in what order.

```
view schematic "System Overview" {
    grid 20;

    include pack at (2, 5) ports {
        east: hv_pos, hv_neg, can_h, can_l;
    };

    include inv at (15, 2) ports {
        west:  dc_pos, dc_neg, can_h, can_l;
        east:  phase_u, phase_v, phase_w;
        south: enable;
    };

    include conv at (15, 18);   // bare box: no ports shown
}
```

Format:

- `view <kind> "<title>" { ... }` — `kind` is `schematic` or `harness`.
- `grid <n>;` — pixels per grid cell. Optional; a sensible default applies.
- `include <name> at (x, y) [ports { ... }];` — place a component by its instance name in the surrounding scope, at grid-cell coordinates. The trailing `;` is always required.
- `ports { <side>: <port>, <port>; ... }` — optional. Each line lists the ports on one `side` (`north`, `east`, `south`, `west`), in the order they should appear on that edge. A `west: a, b;` line puts `a` above `b` on the west edge. List the same side more than once and the lines concatenate.

**The `ports` block controls both layout and scope.** A port is drawn only if it's listed; everything else is hidden. An include with no `ports` block is a bare labelled box.

**Wires are NOT listed in views.** A wire from the model is drawn only when *both* of a segment's endpoints are listed ports on included components; otherwise that segment drops silently. So a listed port whose wire goes to an unlisted (or excluded) port shows as a bare stub with no line.

Sides are authored, never guessed: place a port on the side facing the box it wires to, and line the two boxes up so the wire runs straight (ports sit two grid steps apart, centred on each side). Private (non-`pub`) ports cannot be placed in a view, the same as they cannot be wired from outside.

### Harness views

A `harness` include selects a whole **connector** by its designator and draws it as a pin table. Cables between connectors are derived from the model's wires.

```
view harness "Main HV harness" {
    grid 20;

    include front.hv   at (4, 8);     // <instance>.<connector designator>
    include rear.hv     at (4, 22);
    include inv.hv      at (34, 8);
    include charger.hv  at (40, 22);
}
```

- `include <instance>.<connector> at (x, y);` — the target is `instance.connector` (the connector's designator on that instance's type). The whole connector is drawn: every pin, in pin order. **No `ports { }` block** (it's a whole-connector view), and **no side/facing** — pin facing is auto-oriented from where the cables go.
- A connector must have a designator to be included (see Connector blocks). Including the same instance's other connectors is just more `include` lines.

**Cables are derived, like schematic wires.** A wire renders as a cable strand only when *both* of its endpoints land on *included* connectors; ends on connectorless ports, excluded connectors, or the parent's own ports drop silently. So connectorize the external `pub port`s you want to see in a harness (wrap them in a named `connector` block).

A wire that belongs to a declared `cable` (see Cables) draws WireViz-style: a labelled cable box sits between the two connectors it spans, titled with the cable's label and its `type · length`, one coloured strand per conductor. Loose wires between the same connector pair instead bundle into a plain strand group, each strand drawn in its `color` and annotated with its `label` and gauge.

Views live in the same file as the component they primarily document — like unit tests in Rust source. A view of the system overview belongs in `main.wb` where `vehicle` is defined. A view of the battery pack detail belongs in the file where `battery_pack` is defined. This keeps views close to the data they describe.

## Naming conventions

- **Types and instances both use snake_case.** `cell_module` is a type; `cell_module_1` is an instance. The grammar disambiguates by position.
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