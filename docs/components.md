# Components

## Components

A component is a **type**: a definition of a thing that can be placed in
the design, any number of times. It has an interface, its `pub` ports, and
optionally an implementation: instances of other components and the wires
between them.

```wb
component Contactor {
    pub port in     "IN";
    pub port out    "OUT";
    pub port coil_p "COIL+";
    pub port coil_n "COIL-";
}
```

A component may define other components inside its body. A nested
definition is private to the component that holds it: only that body can
place it, and no `use` can import it.

```wb
component BatteryPack {
    component CellStack {
        pub port pos "+";
        pub port neg "-";
    }

    stack: CellStack;

    pub port hv_pos "HV+";
    pub port hv_neg "HV-";

    wire orange 50 [hv_pos, stack.pos];
    wire orange 50 [hv_neg, stack.neg];
}
```

Two definitions of one name in one scope are `duplicate_type`. A component
that places itself, directly or through others, is `containment_cycle`.

## Ports

```wb component
pub port can_h "CAN H";   // part of the interface
port test      "TEST";    // only the component's own body sees it
```

`[pub] port <name> "<label>";` declares a port: a point a wire can end on.
The name is what wires and views refer to; the label is what the drawing
shows. Port names are unique across the whole component, connectors
included (`duplicate_port`).

A `pub` port can be wired from outside, as `<instance>.<port>`. A port
without `pub` cannot (`private_port`): the component's own wires reach it
by its bare name.

Visibility does not pass through levels. A port of a nested instance is
not a port of the component holding it; to offer it outside, declare a
`pub` port and wire it through, as `BatteryPack` does above with
`hv_pos`.

### Pins

A port may name the connector pin, or pins, it sits on:

```wb component
connector x1 "Deutsch DT06-6S" {
    pub port b_pos "B+"  pin 1;
    pub port gnd   "GND" pins [2, 3];   // ganged for current
    pub port can_h "CAN H" pin 5;
}
```

`pin N` names one pin and `pins [N, …]` several. Pins are positive whole
numbers (`invalid_pin` for `0`), and a pin belongs to one port of a
connector (`duplicate_connector_pin`). A pin is physical data for harness
and pinout views and wire lists; it does not change what is connected.

Pins belong to ports in a [connector](connectors.md). A port outside any
connector may still name a pin, which `check` warns about
(`bare_port_pin`).

## Instances

```wb
component Contactor {
    pub port in  "IN";
    pub port out "OUT";
}

component Vehicle {
    c_main:  Contactor "Main contactor";
    c_spare: Contactor;
}
```

`<name>: <Type> ["<label>"];` places a component. The type is a component
defined in the same file, imported with `use`, or nested in the enclosing
component (`undefined_type` otherwise). The name is what wires and views
refer to; the optional label is what the drawing shows, the name when it
is left out. Instance names are unique within the component
(`duplicate_instance`).

## Part data

A component may say what it is bought as, with `key: "value";` lines among
its members:

```wb
component Contactor {
    part:         "EV200AAANA";
    manufacturer: "TE Connectivity";
    description:  "HV main contactor, 12 V coil";
    rating:       "500 A";

    pub port in  "IN";
    pub port out "OUT";
}
```

- `part` — the manufacturer's part number.
- `manufacturer` — who makes it.
- `description` — what it is.
- `rating` — free text such as `"500 A"` or `"12 V, 2 W"`.

All are optional, each may be set once (`duplicate_cable_property`, also
across the parts of a component split with
[`extend`](projects.md#extend)), and every value is a string
(`cable_property_type`). Any other key is `unknown_cable_property`. The data
belongs to the type, so every placement carries it; the bill of materials
lists it.

What follows the colon tells part data from an instance: a string (or a
number) makes a part-data line, a type name an instance. `part: Fuse;`
places a `Fuse` named `part`.
