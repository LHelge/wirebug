# The wirebug language

wirebug describes an electrical system in `.wb` files: the components it is
built from, the wires between them, and the views that draw it. This
directory is the reference for the language, one topic per page.

| Page | What it covers |
|---|---|
| [Syntax](syntax.md) | Comments, statements and blocks, names, strings and numbers, reserved words |
| [Projects and files](projects.md) | `wirebug.toml`, `main.wb`, `use`, `fragment`, splitting a component with `extend` |
| [Components](components.md) | `component`, nested definitions, ports and pins, instances, part data |
| [Connectors](connectors.md) | `connector` blocks, `connector_type` with grid and face layouts, `inline` connectors |
| [Wires and cables](wires-and-cables.md) | `wire`, colors and gauges, wire properties, lengths, `cable`, `twisted`, `shield`, `splice` |
| [Views](views.md) | `view`, its subject, `grid`, schematic includes and `ports`, `enclosure`, `text`, harness and pinout includes |

## A first file

```wb
component System {
    component Sensor {
        pub port out "OUT";
    }

    component Controller {
        pub port sense "SENSE";
    }

    sensor: Sensor     "Sensor";
    ctrl:   Controller "Controller";

    wire blue 0.5 "SIGNAL" [sensor.out, ctrl.sense];
}

view schematic "Overview" {
    grid: 20;

    include sensor at (4, 4) ports {
        east: out;
    }

    include ctrl at (16, 4) ports {
        west: sense;
    }
}
```

`wirebug init` writes a project like this one; `wirebug check` reports what
is wrong with a project and `wirebug render` draws its views. The README at
the root of the repository covers the commands.

## Every statement at a glance

Where a statement may appear, and the page that describes it.

| Statement | Where | Page |
|---|---|---|
| `use <Name> from "<path>";` | top level | [Projects](projects.md#use) |
| `fragment "<path>";` | top level | [Projects](projects.md#fragment) |
| `component <Name> { … }` | top level, or inside a component | [Components](components.md#components) |
| `extend <Name> { … }` | top level | [Projects](projects.md#extend) |
| `connector_type <Name> "<description>" { … }` | top level | [Connectors](connectors.md#connector-types) |
| `view <kind> "<title>" [for <Name>] { … }` | top level | [Views](views.md) |
| `[pub] port <name> "<label>" [pin N \| pins [N, …]];` | component, connector, inline | [Components](components.md#ports) |
| `<name>: <Type> ["<label>"];` | component | [Components](components.md#instances) |
| `<key>: "<value>";` | component | [Components](components.md#part-data) |
| `connector <name> ["<description>"] { … }` | component | [Connectors](connectors.md#connector-blocks) |
| `connector <name>: <Type> { … }` | component | [Connectors](connectors.md#typed-connectors) |
| `inline <name> ["<label>"] { … }` | component | [Connectors](connectors.md#inline-connectors) |
| `wire <color>[/<tracer>] <gauge> ["<label>"] [<end>, …];` | component, cable, twisted | [Wires and cables](wires-and-cables.md#wires) |
| `cable <name> ["<label>"] { … }` | component | [Wires and cables](wires-and-cables.md#cables) |
| `twisted { … }` | cable | [Wires and cables](wires-and-cables.md#twisted-pairs) |
| `shield ["<label>"] [<end>, …];`, `shield ["<label>"];` | cable, twisted | [Wires and cables](wires-and-cables.md#shields) |
| `splice <name> ["<label>"];` | component | [Wires and cables](wires-and-cables.md#splices) |
| `layout grid { … }`, `layout face { … }` | connector type | [Connectors](connectors.md#pinout-layouts) |
| `grid: <n>;` | view | [Views](views.md#grid) |
| `include … at (x, y) …` | view | [Views](views.md#schematic-views) |
| `enclosure { … }` | view | [Views](views.md#enclosure) |
| `text "<label>" at (x, y);` | view | [Views](views.md#text-boxes) |
