# wirebug

Text-defined electrical schematics and wiring harnesses.

**wirebug** describes an electrical system in a small text DSL (`.wb` files) and turns it into SVG schematics and (eventually) wiring harness drawings. One model, many views — inspired by [Structurizr](https://structurizr.com/) / [LikeC4](https://likec4.dev/) for system architecture and [WireViz](https://github.com/wireviz/WireViz) for wiring harnesses.

A project is a directory rooted at a `main.wb`. Components are *types* with `pub` ports; you instantiate them, wire the instances together, and split a system across files with `use`. The full language is documented in `.github/skills/wirebug-dsl/`.

## Why

When designing an EV conversion — or any electrical system that isn't a PCB — traditional EDA tools like KiCad are overkill, and iterating in a GUI gets tedious. WireViz solved this beautifully for harness drawings. wirebug brings the same text-defined, git-friendly, regenerable-from-source workflow to system-level schematics, and ties schematics to harness drawings via a shared model so the two stay consistent by construction.

## Status

Early and experimental. Expect breaking changes.

What works today:

- The `.wb` DSL front end, end to end via `wirebug check`:
  - a lexer and [`chumsky`](https://github.com/zesterer/chumsky) parser for the DSL;
  - multi-file projects — discover `main.wb`, resolve `use` imports transitively;
  - name resolution (types, instances, ports, view includes);
  - elaboration of the type/instance hierarchy into a flat, addressable IR;
  - validation (undefined names, duplicates, private-port access, containment cycles, …);
  - rich diagnostics via [`miette`](https://github.com/zkat/miette) — source snippets, carets, `--format json`.
- A schematic renderer (rectangle blocks, labeled ports, auto-routed wires, single SVG out) driven straight off the elaborated IR — `.wb` projects render directly.

In transition / not yet:

- Harness drawings (WireViz-style via Graphviz)
- BOM views
- View composition / `extends`
- Unconnected-port linting, theming, manifest emission

## Example

A leaf component (one file), and a top-level component that wires two instances together:

```
// components/battery.wb
component battery {
    pub port hv_pos "HV+";
    pub port hv_neg "HV-";
}
```

```
// main.wb
use battery  from "components/battery.wb"
use inverter from "components/inverter.wb"

component vehicle {
    battery  pack "HV Battery";
    inverter inv  "Motor Controller";

    // shared HV bus: a multi-endpoint wire is a shared rail
    wire orange 50 [pack.hv_pos, inv.dc_pos];
    wire orange 50 [pack.hv_neg, inv.dc_neg];
}

// a view lives next to the component it documents
view schematic "HV Power Path" {
    grid 20;
    include pack at (5, 5) ports {
        east: hv_pos, hv_neg;
    };
    include inv at (20, 5) ports {
        west: dc_pos, dc_neg;
    };
}
```

Check the project — parse, resolve, elaborate, and validate the whole `use` graph:

```sh
wirebug check                 # discovers main.wb by walking up from the CWD
wirebug check examples/main.wb
wirebug check --strict --format json examples/main.wb
```

Problems are reported with source snippets and carets (via miette); a clean run exits 0.

## Concepts

**Component** — a *type*, not an instance: a named definition with `pub` ports (its interface) and, optionally, instances of other components plus the wires between them (its implementation). Components can nest; nested definitions are private to their parent.

**Port** — a named connection point with a human-readable label. `pub` exposes it to instantiators; visibility does not propagate automatically — a parent re-exports by declaring its own `pub` port and wiring it through.

**Connector** — physical grouping metadata (a part description and pin assignments). It is *not* a namespace: a port `c0` inside a connector is still referenced as `instance.c0`, and port names are unique across the whole component.

**Instance** — a placement of a component type, with a name (used in wires) and an optional label (shown in diagrams).

**Wire** — a colour, a gauge (mm²), and two or more endpoints (`instance.port`, or a bare `port` for the enclosing component's own port). Multi-endpoint wires model shared rails and T-junctions.

**View** — a rendering target that documents a component: a kind (`schematic` for now), a grid, and which instances to place where, each with the ports to show on each side. Wires are derived from the model, never listed in views — a wire draws only between ports both views list.

**Project** — a directory rooted at `main.wb`, with a `wirebug.toml` manifest beside it (see below). Logical hierarchy comes only from `use` imports and DSL nesting, never from directory layout.

## Project manifest

Every project carries a `wirebug.toml` beside `main.wb`. It's a small TOML file with a single `[project]` table — `name` and `version` are required, the rest are optional:

```toml
[project]
name        = "aphid-evpack"
version     = "0.1.0"
description = "Aphid EV conversion — top-level vehicle wiring"
authors     = ["Aphid EV team"]
license     = "MIT"
revision    = "B"             # optional; auto-filled from git when absent
date        = "2026-05-28"    # optional; ISO date
```

`name` and `version` appear in the rendered output: as a small stamp in the bottom-right corner of every SVG, and as the page header on the HTML index. `revision` and `date`, when set, are appended to the stamp.

**Revision from git.** If `revision` is omitted, wirebug shells out to `git rev-parse --short HEAD` in the project directory and uses the result, suffixed `-dirty` when the working tree has uncommitted changes. An explicit `revision = "..."` in the manifest always wins. If `git` isn't on `PATH`, or the directory isn't a git repo, the field stays empty and the stamp simply omits it — no error.

**Embedding into another document.** Pass `--embed` to `wirebug render` to emit SVGs intended for inclusion in another page, report, or static site. Embed-mode SVGs drop the built-in `<style>` block (the host stylesheet owns the look), suppress the bottom-right project-identity stamp, and class-tag the root `<svg>` with `wirebug wirebug-schematic` (or `wirebug-harness`) so host CSS can scope rules under `.wirebug`. The HTML index is replaced by a `manifest.json` sidecar listing every view (title, filename, kind) plus the project's identity, so a downstream build can enumerate and embed views without parsing each SVG. (The previous `--no-stamp` flag is removed: embedding was its only use case, and `--embed` now expresses it directly.)

## Rendering

`render` runs the same DSL pipeline as `check`, then draws every view in the design to its own SVG. Each `include` position is in **grid units**: the renderer multiplies by the grid step, and `x`/`y` are the box **centre**. Ports sit two grid steps apart and are centred on each side, so lining two components up makes the wire between them run straight. A box sizes itself from its busiest side (there is no explicit size in the DSL); omit `grid:` for the default. The grid must be coarse enough that the two-step port pitch clears a label — too fine a grid errors rather than overlapping labels.

The view authors each port's side and order directly in its `ports` block, and that listing is also the scope: a box shows exactly the ports it lists, and a wire draws only where both ends are listed. Place a port on the side facing the box it connects to.

```sh
wirebug render                       # discovers main.wb by walking up from the CWD
wirebug render examples/main.wb --out out/           # one SVG per view, into out/
wirebug render examples/main.wb --out out/ --embed   # naked SVGs + manifest.json for embedding
```

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