# wirebug

Text-defined electrical schematics and wiring harnesses.

**wirebug** reads a YAML description of an electrical system and renders it as SVG schematics and (eventually) wiring harness drawings. One model, many views — inspired by [Structurizr](https://structurizr.com/) / [LikeC4](https://likec4.dev/) for system architecture and [WireViz](https://github.com/wireviz/WireViz) for wiring harnesses.

## Why

When designing an EV conversion — or any electrical system that isn't a PCB — traditional EDA tools like KiCad are overkill, and iterating in a GUI gets tedious. WireViz solved this beautifully for harness drawings. wirebug brings the same text-defined, git-friendly, regenerable-from-source workflow to system-level schematics, and ties schematics to harness drawings via a shared model so the two stay consistent by construction.

## Status

Early and experimental. Expect breaking changes.

What works today:

- Parsing a YAML model (components, ports, connections)
- Parsing a schematic view (which components to show, where to place them)
- Rendering rectangle-style component blocks with labeled ports
- Drawing wires between ports
- Emitting a single SVG

Not yet:

- Harness drawings (WireViz-style via Graphviz)
- BOM views
- View composition / `extends`
- Validation beyond referential integrity
- Theming
- Manifest emission for downstream tools

## Example

A minimal model and view:

```yaml
# model.yaml
components:
  pack:
    label: "Aphid 96V Pack"
    ports:
      hv_pos: right
      hv_neg: right

  inverter:
    label: "Curtis 1238"
    ports:
      dc_pos: left
      dc_neg: left
      u: right
      v: right
      w: right

connections:
  - { from: pack.hv_pos, to: inverter.dc_pos }
  - { from: pack.hv_neg, to: inverter.dc_neg }
```

```yaml
# views/hv_overview.yaml
kind: schematic
title: "HV Power Path"
include: [pack, inverter]
layout:
  pack:     { x: 0,   y: 0 }
  inverter: { x: 400, y: 0 }
```

```sh
wirebug render --model model.yaml --view views/hv_overview.yaml --out hv.svg
```

## Concepts

**Model** — the source of truth. Components, their ports, and connections between them. Lives in one or more YAML files.

**Component** — a named block with named ports. For now, components render as labeled rectangles; later, hierarchical sub-systems will also be expressible as components.

**Port** — a connection point on a component side (`north`, `south`, `east`, `west`). Referenced as `component.port`.

**Connection** — a link from one port to another. Will grow to carry gauge, color, harness assignment, and length.

**View** — a renderable subset of the model with a layout and a chosen renderer. Different view kinds (`schematic`, eventually `harness` and `bom`) use different renderers; all consume the same underlying model.

## Design principles

- **Plain text in, SVG out.** No GUI, no proprietary formats. `git diff` should be meaningful.
- **One model, many views.** Define the system once; render it from many angles at different levels of detail.
- **Manual layout, by choice.** Auto-layout produces auto-looking results. For a build log where each diagram is a deliberate artifact, manual coordinates win.
- **Composable with other tools.** wirebug emits artifacts (SVGs plus a manifest); downstream tools — a static site generator, a BOM aggregator — can consume them without coupling.

## Connector routing

Wires are routed automatically so they avoid component boxes and use only
right-angle bends. The algorithm is the object-avoiding orthogonal
connector routing behind [libavoid](https://www.adaptagrams.org/documentation/libavoid.html):

> Michael Wybrow, Kim Marriott, and Peter J. Stuckey. "Orthogonal
> Connector Routing." In *Graph Drawing (GD 2009)*, LNCS 5849,
> pp. 219–231. Springer, 2010.
> [[PDF]](https://people.eng.unimelb.edu.au/pstuckey/papers/gd09.pdf)

wirebug implements all three stages — building an orthogonal visibility
graph, finding minimum-bend routes through it with A\*, and nudging apart
wires that share a channel so a bundle draws as distinct parallel lines.

## Build

```sh
cargo build --release
```

## License

MIT.