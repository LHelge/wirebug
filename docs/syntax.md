# Syntax

The surface of the language borrows from Rust: `//` comments, `;` at the
end of a statement, brace blocks, `name: Type` for an instance, `pub` for
what a component shows to the outside.

## Comments

`//` starts a comment that runs to the end of the line. There are no block
comments.

```wb
// A relay with its coil and one contact.
component Relay {
    pub port coil "COIL";   // the coil's switched side
    pub port com  "COM";
}
```

## Statements and blocks

A statement ends with `;`. A block (`{ … }`) ends at its closing brace, with
no `;` after it, even where the block takes the place of a statement's `;`
(a wire with a property body, an include with a `ports { }` block).

Lists between `[ ]` (pins, wire ends, shield ends) and the port lists of a
`ports { }` block are separated by commas and may end with one, so a list
split over several lines diffs cleanly:

```wb component
pub port gnd "GND" pins [
    2,
    3,
];
```

Statements in a block may come in any order, unless a page says otherwise.

## Names

A name is a run of ASCII letters, digits and `_` that does not start with
a digit. Names are case-sensitive. By convention types (components,
connector types) are `CamelCase` and everything else (instances, ports,
connectors, cables, splices) is `snake_case`; the language does not
enforce it.

A dot joins two names in a reference: `inv.dc_pos` is the port `dc_pos` of
the instance `inv`. A reference has at most two parts.

## Strings

A string is written between double quotes and holds one line: `"HV+"`,
`"120 Ω termination"`. It has no escapes, so it cannot hold a `"`. Strings
label things for a reader (ports, instances, wires, views) and carry part
data; nothing in the language refers to a thing by its string.

## Numbers

A number is written in decimal, optionally negative and with a fraction:
`4`, `0.5`, `-2` (not `.5`). There is no arithmetic.

Only a [length](wires-and-cables.md#lengths) takes a unit, written directly
after the number with no space (`150mm`). A unit on any other number (a
gauge, a pin, a coordinate, a `grid:`) is a syntax error. Since a unit is
whatever name touches the digits, a number followed directly by a name is
one token: write `cavity 1 at (0, 0)`, never `cavity 1at (0, 0)`.

## Reserved and contextual words

The words that declare the design are **reserved**: they never name
anything.

`use` `component` `extend` `connector_type` `connector` `inline` `port`
`pub` `pin` `pins` `wire` `cable` `twisted` `view`

Using one as a name is the `reserved_word` error:

```wb component error
pub port wire "W";   // `wire` is reserved: name it `wire_1` or `w`
```

The other keywords are **contextual**: they mean something only where
their statement can start, and are ordinary names everywhere else.

`from` `fragment` `shield` `splice` `for` `include` `text` `at` `grid`
`enclosure` `ports` `layout` `face` `cavity` `rows` `cols` `numbering` `size`

```wb component
pub port size   "SIZE";
pub port shield "SHIELD";
grid: Sensor;            // an instance named `grid`
```

Words the grammar reads in one place only are no keywords at all: the view
kinds (`schematic`, `harness`, `pinout`), the property keys (`type`,
`length`, `part`, `manufacturer`, `description`, `rating`), the inline
halves (`male`, `female`), the sides (`north`, `east`, `south`, `west`),
the wire colors, the grid numbering modes and the cavity size `large`.
