# Views

A view is a drawing of one component, its **subject**. It says what to
place where; the wires come from the model. There are three kinds:

- `schematic` — the subject's instances as boxes, with the ports the view
  lists on the sides it says, and the wires between them;
- `harness` — connectors as pin tables, with the cables and wires between
  them: the build drawing of a loom;
- `pinout` — the subject's own connectors: each cavity face and a crimping
  table of what goes in each pin.

```wb
component Lamp {
    connector x1 "Lamp 2p" {
        pub port feed "FEED" pin 1;
        pub port gnd  "GND"  pin 2;
    }
}

view pinout "Lamp pinout" for Lamp {
    include x1 at (0, 0);
}
```

`view <kind> "<title>" [for <Component>] { … }` is a top-level item. Any
other kind is `unknown_view_kind`. Titles are unique across the project
(`duplicate_view_title`, a warning; the repeat is drawn to a file of its
own). The body holds, in any order, at most one `grid:`, at most one
`enclosure { }` (`duplicate_view_item`), and any number of `include`s and
`text` boxes.

## The subject

Without `for`, a view documents the single top-level component of its file
(in a file of `extend`s, the component extended). In a file with no or
several top-level components it must name one (`view_subject`), with
`for <Component>`: a component the file defines at its top level or
imports with `use`. A name that is not in scope is `undefined_type`, a
connector type `view_subject_not_component`.

Every include of the view resolves against the subject, and the view is
drawn against a placement of it: a view of a component the design never
places is `uninstantiated_view_subject`.

## Grid

```wb view
grid: 20;
```

`grid: <n>;` is the size of one grid cell in drawing units; every position
in the view counts cells. It is optional. It must be positive
(`invalid_grid`), and in a schematic at least 7.5, so that ports two steps
apart clear each other's labels (`grid_too_fine`).

A view may span at most 8192 units on a side (`view_too_large`); split a
large drawing into several views.

## Schematic views

```wb view
include pack at (2, 5) ports {
    east: hv_pos, hv_neg;
}

include inv at (20, 2) ports {
    west:  dc_pos, dc_neg;
    east:  phase_u, phase_v, phase_w;
    south: enable;
}

include conv at (20, 18);
```

`include <instance> at (<x>, <y>) [ports { … }]` places an instance of the
subject with its box centred on the grid point (`x` to the right, `y` down;
negative is fine). Without a `ports` block it ends with `;` and draws a
bare box; with one it ends at the block's `}`. Only instances can be
included: an inline connector or a splice is `wrong_include_form`, and an
instance included twice `duplicate_view_include`.

Each line of the `ports` block, `<side>: <port>, …;`, puts ports on one
side, `north`, `east`, `south` or `west` (`unknown_port_side` otherwise),
in the order written: top to bottom on `west` and `east`, left to right on
`north` and `south`. A side may have several lines; they join. Only `pub`
ports can be listed, each once per include (`duplicate_view_port`).

The listing is also the scope: a box shows only the ports it lists, and a
wire is drawn only between two listed ports. A listed port whose wire goes
somewhere the view does not show ends in a short stub that names where it
goes (`→ vcm.can_h`).

A splice or an inline connector is never drawn in a schematic. The ports
wired to it show connected instead: a signal that runs from one device to
an inline and on from the inline to another is one wire between the two
devices, in the colour of the conductor that reaches the second one. Where
the view leaves the far side out, the stub names the splice or the
inline's port, then what lies beyond it (`→ ic_pedal.gnd (pedal.gnd)`).

Boxes may touch but not overlap (`overlapping_includes`). A wire leaves its
port by one grid step and keeps one step clear of other boxes, so a wired
port needs two grid steps of room in front of it (`include_too_close`), and
a wire walled in on every side is `unroutable_wire`. A gap of two steps
between boxes holds one wire, and each further wire through it needs two
steps more: with less, two wires are drawn on one line, and when they
belong to different nets `check` warns that they cannot be told apart
(`overlapping_wires`).

### Enclosure

An `enclosure` block draws the subject itself, as a dashed box around the
view, with some of the subject's own ports on its edge:

```wb
component Cell {
    pub port pos "+";
    pub port neg "-";
}

component Module {
    pub port hv_pos "HV+";
    pub port hv_neg "HV-";
    pub port sense  "SENSE";

    cell: Cell;

    wire orange 16 [hv_pos, cell.pos];
    wire orange 16 [hv_neg, cell.neg];
}

view schematic "Module" for Module {
    enclosure {
        hv_pos at (west, 4);
        hv_neg at (east, 4);
        sense  at (2, north);
    }

    include cell at (10, 4) ports {
        west: pos;
        east: neg;
    }
}
```

`enclosure { <port> at (<x>, <y>); … }` places a `pub` port of the subject
on the edge. One coordinate is a side and pins the port to that edge: `west`
or `east` in the `x` slot, `north` or `south` in the `y` slot; the other is
a grid coordinate along the edge (`enclosure_anchor` otherwise). The box
wraps the includes by itself. A wire between an own port the enclosure
places and a listed port inside is drawn; an own port it does not place
shows as the stub `→ <port> (leaves <Subject>)`. An empty `enclosure { }`
draws the box with no ports.

### Text boxes

```wb view
text "120 Ω termination" at (8, 4);
```

`text "<label>" at (<x>, <y>);` draws a note in a small box centred on the
grid point. It has no name, and only a schematic takes one
(`unsupported_view_text`). `check` does not test text boxes for overlap:
keep them clear of the boxes and wires.

The older form names the box and puts the label last. It still draws, but
warns (`deprecated_text_syntax`) and will be removed:

```wb view
text note at (8, 4) "120 Ω termination";   // deprecated: write the label first
```

## Harness views

```wb view
include ecu.x1     at (0, 0);
include bms.x2     at (30, 0);
include sp_gnd     at (15, 10);
include ic_pedal.female at (30, 12);
```

A harness include, `include <target> at (<x>, <y>);`, places one of:

- `<instance>.<connector>` — a connector of an instance of the subject,
  drawn as a pin table (`unknown_connector` when the instance has no such
  connector);
- `<inline>.male` or `<inline>.female` — one half of an inline connector
  of the subject, the half built onto the loom this view draws; the half
  must be declared on the inline (`undeclared_inline_half`), and a view
  includes at most one half of an inline (`duplicate_view_include`);
- `<splice>` — one of the subject's splices, drawn as a dot.

It takes no `ports` block (`wrong_include_form`): a pin table shows the pins
that carry a conductor in the view. A view includes each target once
(`duplicate_view_include`). A conductor is drawn when the view
includes what both its ends land on; a cable's conductors run through its
cable box.

Once a component has a harness view, `check` warns about every conductor
between two of its children's connectors, or to one of its splices, that
none of its harness views draws (`undrawn_conductor`).

## Pinout views

```wb view
include hv      at (0, 0);
include control at (14, 22);
```

A pinout include, `include <connector> at (<x>, <y>);`, names a connector of
the subject itself, not of an instance: `include control`, not
`include inv.control`. It draws the connector's face from its connector
type's [layout](connectors.md#pinout-layouts), seen from the harness side,
above a table of each pin's port and the conductors crimped into it. It
takes no `ports` block.
