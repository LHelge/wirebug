# Connectors

A connector groups the ports of a component that sit in one physical
housing, and says which pin each one is on. It is physical data for the
harness and pinout views, the wire list and the bill of materials: it is
not a namespace. A port inside a connector is still referred to as
`<instance>.<port>`, never through the connector's name.

There are three forms: a connector block described in place, a connector
of a reusable `connector_type`, and an `inline` connector that joins two
harnesses.

## Connector blocks

```wb
component Ecu {
    connector x1 "Deutsch DT06-6S" {
        pub port b_pos "B+"    pin 1;
        pub port gnd   "GND"   pins [2, 3];
        pub port can_h "CAN H" pin 5;
        pub port can_l "CAN L" pin 6;
    }

    connector diag {
        pub port k_line "K" pin 1;
    }
}
```

`connector <name> ["<description>"] { <port>* }`. The name, the connector's
**designator**, is required: it is how harness and pinout views include
the connector (`include ecu.x1`). Designators are unique within a
component (`duplicate_connector_name`). The description, which is
optional, says what the housing is, for a reader.

The body holds [ports](components.md#ports) and nothing else; `pub` and
private ports may mix. A connector may leave a port without a pin, when
the pin is not known yet.

## Connector types

A `connector_type` describes a housing once, so many components can use
it:

```wb
connector_type Dt06_6s "Deutsch DT06-6S" {
    part: "DT06-6S";

    layout grid {
        rows: 2;
        cols: 3;
    }
}
```

`connector_type <Name> "<description>" { … }` is a top-level item, and
`use` imports it like a component. The description is required. The body
holds `key: value;` lines, whose value is a string or a number, and at most
one [layout](#pinout-layouts). `part` is the housing's part number, shown
on harness and pinout drawings and in the bill of materials; other keys are
kept as notes.

### Typed connectors

A component places a connector type with the same body as a connector
block:

```wb
connector_type Dt06_6s "Deutsch DT06-6S" {
    part: "DT06-6S";
}

component Ecu {
    connector x1: Dt06_6s {
        pub port b_pos "B+"  pin 1;
        pub port gnd   "GND" pins [2, 3];
    }
}
```

`connector <name>: <Type> { <port>* }`. The name is the designator; the
type must be a connector type in scope (`undefined_connector_type`). Each
port's pins name cavities of the type's layout.

### Pinout layouts

A layout says where each cavity sits on the connector's face, so a pinout
view can draw it. It is written **as seen from the harness side**, the
side the wires go in: mirror a datasheet drawing of the mating face before
copying it.

A rectangular connector takes a grid:

```wb
connector_type Dual12 "Dual row 12p" {
    layout grid {
        rows: 2;
        cols: 6;
        numbering: odd_even;
    }
}
```

`layout grid { rows: N; cols: N; [numbering: <mode>;] }`, in this order.
`rows` and `cols` are whole numbers. The numbering says how pin numbers walk
the grid:

- `row_major` (the default) — left to right along each row, then the next
  row down.
- `odd_even` — down each column first, so a 2×N connector has its odd pins
  on one row and its even pins on the other.
- `clockwise` — around the face, clockwise.
- `counter_clockwise` — around the face, counter-clockwise; on a 2×N
  connector the upper row is mirrored and the lower row reads left to
  right.

Any other connector takes a face, one line per cavity:

```wb
connector_type InverterControl "Inverter control 6p" {
    layout face {
        cavity 1 at (0, 0) size large;
        cavity 2 at (2, 0) size large;
        cavity 3 at (4, 0);
        cavity 4 at (5, 0);
        cavity 5 at (4, 1);
        cavity 6 at (5, 1);
    }
}
```

`layout face { cavity <pin> at (<x>, <y>) [size large]; … }`, at least one
cavity. Coordinates are slots on the face, `x` to the right and `y` down; a
cavity fills one slot, and `size large` fills two by two. The drawing starts
at the first occupied slot, so the face may start anywhere.

A connector type without a layout still works everywhere; its pinout view
draws the pin table without a face.

## Inline connectors

An inline connector is a mated plug and receptacle in the middle of a
harness, where one loom hands over to another. It is one set of ports with
two housing halves: pin N on one half mates with pin N on the other.

```wb
connector_type Dt04_3p "Deutsch DT04-3P" {
    part: "DT04-3P";
}

connector_type Dt06_3s "Deutsch DT06-3S" {
    part: "DT06-3S";
}

component Vehicle {
    inline ic_pedal "Pedal branch" {
        male:   Dt04_3p;
        female: Dt06_3s;

        port apps1 "APPS1" pin 1;
        port apps2 "APPS2" pin 2;
        port gnd   "GND"   pin 3;
    }
}
```

`inline <name> ["<label>"] { … }` is a member of a component. Its body
holds, in any order:

- `male: <Type>;` and `female: <Type>;` — the connector type of each half,
  each optional and at most once (`duplicate_inline_half`). Any other key is
  `unknown_inline_property`.
- Ports, written as in a connector block, without `pub`: the ports of an
  inline are reachable anywhere in its component, so `pub` means nothing
  and warns (`inline_pub_port`).

An inline is referred to like an instance: a wire ends on
`ic_pedal.apps1`, and its name may not repeat an instance's
(`duplicate_instance`). The loom on each side runs its conductors to the
same ports, which is what makes the two looms continuous.

A harness view includes one half, the one built onto the loom it draws
(`include ic_pedal.female at (x, y);`): see
[Harness views](views.md#harness-views).
