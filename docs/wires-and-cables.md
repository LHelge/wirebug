# Wires and cables

Wires say what is connected. They are members of a component body and run
between the ports of the instances it places, its own ports and its
splices. Views never list wires: each view draws the wires between what it
shows.

## Wires

```wb component
wire orange 50 [pack.hv_pos, inv.dc_pos];
wire red 0.5 "CAN H" [ecu.can_h, bms.can_h];
wire green/yellow 2.5 "PE" [charger.pe, chassis];
```

`wire <color>[/<tracer>] <gauge> ["<label>"] [<end>, <end>, …];`

- **Color** — one of the IEC 60757 colors: `black`, `brown`, `red`,
  `orange`, `yellow`, `green`, `blue`, `violet`, `grey`, `white`, `pink`,
  `turquoise`, `gold`, `silver`; `purple` and `gray` mean `violet` and
  `grey`. Any other name still draws, in that name, with the
  `unknown_wire_color` warning. A two-tone wire writes its base color and
  its tracer (stripe) color with a `/`: `green/yellow`.
- **Gauge** — the cross-section in mm², a positive number without a unit
  (`invalid_gauge` otherwise).
- **Label** — optional: the signal name, shown on the wire in harness
  drawings.
- **Ends** — two or more (`wire_arity` otherwise), each written once
  (`duplicate_endpoint`):
  - `<instance>.<port>` — a `pub` port of an instance, or a port of an
    [inline connector](connectors.md#inline-connectors);
  - `<name>` — a port of the enclosing component itself, or one of its
    [splices](#splices).

A wire with more than two ends is a shared rail (a bus, a ground, a common
enable line): every end is connected, and where the branches meet is left
to the build. When the joint is a physical part of the harness, declare it
as a [splice](#splices) instead. Two separate wires may end on one port: a
double crimp.

### Wire properties

A wire may end in a body of `key: value;` lines in place of its `;`:

```wb component
wire red 0.5 "12V" [fuses.ecu, ecu.b_pos] {
    length: 1.2;
    type:   "FLRY-B";
    part:   "FLRY-B-0.5-RD";
}
```

- `length` — the wire's [length](#lengths). On a rail of more than two
  ends it is the length of the whole rail.
- `type` — the insulation or construction, a string.
- `part` — the part number, a string.

All are optional and each may be set once (`duplicate_cable_property`). Any
other key is `unknown_cable_property`, and a value of the wrong kind (a
string for `length`, a number for `type`) is `cable_property_type`.

### Lengths

A length is a positive number with an optional unit written directly after
it:

```wb component
wire red   1 [ecu.a, lamp.a] { length: 2; }       // metres
wire black 1 [ecu.b, lamp.b] { length: 2m; }
wire black 1 [ecu.c, lamp.c] { length: 80cm; }
wire black 1 [ecu.d, lamp.d] { length: 150mm; }
```

A bare number is in metres; the units are `m`, `cm` and `mm`. Any other unit
is `unknown_length_unit`, a length that is not positive `invalid_length`,
and a space between the number and its unit (`2 m`) a syntax error.

## Cables

A cable is a bundle of conductors that run together from one place to
another, a multi-core or a loom branch, with its construction data. Its
harness drawing is a labelled box between the connectors it joins.

```wb component
cable motor_phases "Motor phases" {
    type:   "Shielded 3-core";
    length: 1.2;

    wire orange 35 "U" [inv.phase_u, mot.phase_u];
    wire orange 35 "V" [inv.phase_v, mot.phase_v];
    wire orange 35 "W" [inv.phase_w, mot.phase_w];
}
```

`cable <name> ["<label>"] { … }` is a member of a component. The name is its
designator, unique within the component (`duplicate_cable_name`); the label
titles its box, the name when it is left out. The body holds, in any order:

- `type: "<string>";` — the cable's construction.
- `length: <length>;` — its length. A conductor with a `length` of its own
  keeps it; the others take the cable's.
- Conductors: `wire`s, each with **exactly two** ends
  (`cable_wire_arity`); a rail stays a loose wire outside any cable.
  A conductor may have a property body like any wire.
- [Twisted pairs](#twisted-pairs).
- At most one [shield](#shields), the cable's overall screen.

A cable without a conductor is the `empty_cable` warning. A cable's
conductors are ordinary wires: schematic views draw them like any other.

### Twisted pairs

```wb component
cable can "CAN" {
    twisted {
        wire white/blue 0.5 "CAN H" [ecu.can_h, bms.can_h];
        wire white/red  0.5 "CAN L" [ecu.can_l, bms.can_l];
    }
    wire black 0.5 "GND" [ecu.gnd, bms.gnd];
}
```

`twisted { <wire> <wire> [<shield>] }` inside a cable groups two conductors
twisted together. It holds exactly two wires; any other number is a syntax
error. Pairs and plain conductors may mix in one cable.

### Shields

A screened cable, or a screened pair, declares its screen with a `shield`
line: in the cable body for the overall screen, in a `twisted { }` group for
the pair's own.

```wb component
cable can2 "CAN2" {
    type: "Shielded twisted pair";

    twisted {
        wire white/blue 0.5 "CAN H" [rear.can_h, vcm.can_h];
        wire white/red  0.5 "CAN L" [rear.can_l, vcm.can_l];
        shield [vcm.shield_gnd];
    }
    shield "SHLD" [vcm.shield_gnd, rear.shield_gnd];
}

cable sensor "Sensor" {
    wire white 0.5 [ecu.sig, sensor.sig];
    shield;
}
```

`shield ["<label>"] [<end>, …];` lists the pins its drain wire is
terminated on, written like wire ends: one end (grounded at one end, the
usual), two (both ends), or none (`shield;`: the screen lands on no pin,
cut back or bonded at a gland). More than two is `shield_arity`, and an
empty `[]` is a syntax error. A shield is no conductor: it has no color or
gauge, and does not count as one of a pair's two wires. A cable or a pair
holds at most one (`duplicate_shield`). A drain lands on a pin, never on a
splice (`shield_on_splice`). The label names the drain in a harness
drawing.

## Splices

A splice is a joint in the harness, a crimp or solder splice, that
conductors end on. Each conductor to it is an ordinary two-ended wire with
its own color, gauge and length, which a rail cannot give each leg.

```wb
component Lamp {
    pub port gnd "GND";
}

component Vehicle {
    left:  Lamp "Left lamp";
    right: Lamp "Right lamp";

    pub port chassis "CHASSIS";

    splice sp_gnd "Ground splice" {
        part:        "D-436-36";
        description: "Solder sleeve";
    }
    splice sp_spare;

    wire black 0.5 "GND" [left.gnd,  sp_gnd];
    wire black 0.5 "GND" [right.gnd, sp_gnd];
    wire black 2.5 "GND" [sp_gnd, chassis];
}
```

`splice <name> ["<label>"];`, or with a body of `part: "<string>";` and
`description: "<string>";` in place of the `;`, is a member of a component.

- A wire ends on a splice by its bare name, as on an own port. Its name may
  not repeat any port, instance, inline, connector, cable or other splice of
  the component (`duplicate_splice_name`).
- Every conductor ending on one splice is joined to the others, including
  cable conductors and a wire from one splice to another.
- A wire to a splice has exactly two ends (`splice_wire_arity`).
- A splice that fewer than two conductors end on joins nothing
  (`splice_joins_nothing`, a warning): `sp_spare` above.
- Schematic views draw no splice: the ports it joins show connected.
  Harness views can place it as a dot
  ([Harness views](views.md#harness-views)).
