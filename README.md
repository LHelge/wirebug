# wirebug

Text-defined electrical schematics and wiring harnesses.

**wirebug** describes an electrical system in a small text DSL (`.wb` files) and turns it into SVG schematics, wiring harness drawings, and connector pinout drawings. One model, many views — inspired by [Structurizr](https://structurizr.com/) / [LikeC4](https://likec4.dev/) for system architecture and [WireViz](https://github.com/WireViz/WireViz) for wiring harnesses.

A project is a directory rooted at a `wirebug.toml` manifest, with `main.wb` as the conventional entry file beside it. Components are *types* with `pub` ports; you instantiate them, wire the instances together, and split a system across files with `use`. The full language is documented in `.agents/skills/wirebug-dsl/` and mirrored in `.claude/skills/wirebug-dsl/`.

## Why

When designing an EV conversion — or any electrical system that isn't a PCB — traditional EDA tools like KiCad are overkill, and iterating in a GUI gets tedious. WireViz solved this beautifully for harness drawings. wirebug brings the same text-defined, git-friendly, regenerable-from-source workflow to system-level schematics, and ties schematics to harness drawings via a shared model so the two stay consistent by construction.

## Status

Early and experimental. Expect breaking changes.

What works today:

- The `.wb` DSL front end, end to end via `wirebug check`:
  - a lexer and [`chumsky`](https://github.com/zesterer/chumsky) parser for the DSL;
  - multi-file projects — discover `wirebug.toml`, load `main.wb`, and resolve `use` imports transitively;
  - name resolution (types, instances, ports, view includes);
  - elaboration of the type/instance hierarchy into a flat, addressable IR;
  - validation (undefined names, duplicates, private-port access, containment cycles, …);
  - rich diagnostics via [`miette`](https://github.com/zkat/miette) — source snippets, carets, `--format json`.
- Three SVG renderers driven straight off the elaborated IR, one file per view plus an HTML index that groups them into Schematics/Harnesses/Pinouts tabs:
  - a **schematic** renderer (rectangle blocks, labeled ports, object-avoiding orthogonal wire routing);
  - a **harness** renderer (WireViz-style pin tables, a central spine, and bezier cable bundles);
  - a **pinout** renderer (connector cavity faces plus pin tables, authored from the harness side).
- `wirebug render` to disk (SVG, or `--png` rasterised, or `--pdf` for a single A4 PDF with one view per page, or `--embed` for naked SVGs + a `manifest.json` sidecar).
- `wirebug serve` — a live-reloading dev server that re-renders on `.wb` or `wirebug.toml` saves.
- A VSCode extension ([`editors/vscode/`](editors/vscode/)) — syntax highlighting plus live diagnostics and context-aware completion via `wirebug lsp`, a language server over stdio.

In transition / not yet:

- Object-avoiding harness routing (the harness renderer flexes beziers, no obstacle avoidance yet)
- BOM views
- View composition / `extends`
- Unconnected-port linting, theming, auto-layout

## Example

A leaf component (one file), and a top-level component that wires two instances together:

```
// components/battery.wb
component Battery {
    pub port hv_pos "HV+";
    pub port hv_neg "HV-";
}
```

```
// main.wb
use Battery  from "components/battery.wb";
use Inverter from "components/inverter.wb";

component Vehicle {
    pack: Battery  "HV Battery";
    inv:  Inverter "Motor Controller";

    // shared HV bus: a multi-endpoint wire is a shared rail
    wire orange 50 [pack.hv_pos, inv.dc_pos];
    wire orange 50 [pack.hv_neg, inv.dc_neg];
}

// a view lives next to the component it documents
view schematic "HV Power Path" {
    grid: 20;
    include pack at (5, 5) ports {
        east: hv_pos, hv_neg;
    }
    include inv at (20, 5) ports {
        west: dc_pos, dc_neg;
    }
}
```

Check the project — parse, resolve, elaborate, and validate the whole `use` graph:

```sh
wirebug check                 # discovers wirebug.toml by walking up from the CWD
wirebug check examples        # point at the project root
wirebug check examples/wirebug.toml
wirebug check --strict --format json examples
```

Problems are reported with source snippets and carets (via miette); a clean run exits 0.

## Concepts

**Component** — a *type*, not an instance: a named definition with `pub` ports (its interface) and, optionally, instances of other components plus the wires between them (its implementation). Components can nest; nested definitions are private to their parent.

**Port** — a named connection point with a human-readable label. `pub` exposes it to instantiators; visibility does not propagate automatically — a parent re-exports by declaring its own `pub` port and wiring it through.

**Connector** — physical grouping metadata (a part description and pin assignments). It is *not* a namespace: a port `c0` inside a connector is still referenced as `instance.c0`, and port names are unique across the whole component. Reusable `connector_type` definitions can carry shared metadata and pinout layout, then components instantiate them with `connector x1: TypeName { pin 1: port; }`.

**Instance** — a placement of a component type, with a name (used in wires) and an optional label (shown in diagrams).

**Wire** — a colour, a gauge (mm²), and two or more endpoints (`instance.port`, or a bare `port` for the enclosing component's own port). Multi-endpoint wires model shared rails and T-junctions.

**View** — a rendering target that documents a component: a kind (`schematic`, `harness`, or `pinout`), a grid, and what to place where. A `schematic` include lists the ports to show on each side; a `harness` include names a child instance's connector and draws its whole pin table; a `pinout` include names one of the subject component's own connectors and draws its cavity face. Wires are derived from the model, never listed in views — a wire draws only between ports/connectors both views include.

**Project** — a directory rooted at `wirebug.toml`, with `main.wb` beside it as the entry file. The CLI intentionally feels Cargo-like: from inside a project, commands walk up parent directories until they find the manifest; you can also point commands at the project root or at `wirebug.toml` explicitly. Logical hierarchy comes only from `use` imports and DSL nesting, never from directory layout.

## Project manifest

Every project carries a `wirebug.toml` at the project root, beside `main.wb`. It's a small TOML file with a single `[project]` table — `name` and `version` are required, the rest are optional:

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

`name` and `version` appear in the rendered output: as a small stamp in the bottom-right corner of every SVG, as the page header on the HTML index, and in every PDF page's footer beside the page number. `revision` and `date`, when set, are appended to the stamp.

**Revision from git.** If `revision` is omitted, wirebug shells out to `git rev-parse --short HEAD` in the project directory and uses the result, suffixed `-dirty` when the working tree has uncommitted changes. An explicit `revision = "..."` in the manifest always wins. If `git` isn't on `PATH`, or the directory isn't a git repo, the field stays empty and the stamp simply omits it — no error.

**Embedding into another document.** Pass `--embed` to `wirebug render` to emit SVGs intended for inclusion in another page, report, or static site. Embed-mode SVGs drop the built-in `<style>` block (the host stylesheet owns the look), suppress the bottom-right project-identity stamp, and class-tag the root `<svg>` with `wirebug wirebug-schematic` (or `wirebug-harness`) so host CSS can scope rules under `.wirebug`. The HTML index is replaced by a `manifest.json` sidecar listing every view (title, filename, kind) plus the project's identity, so a downstream build can enumerate and embed views without parsing each SVG. (The previous `--no-stamp` flag is removed: embedding was its only use case, and `--embed` now expresses it directly.)

## Rendering

`render` runs the same DSL pipeline as `check`, then draws every view in the design to its own SVG. Each `include` position is in **grid units**: the renderer multiplies by the grid step, and `x`/`y` are the box **centre**. Ports sit two grid steps apart and are centred on each side, so lining two components up makes the wire between them run straight. A box sizes itself from its busiest side (there is no explicit size in the DSL); omit `grid:` for the default. The grid must be coarse enough that the two-step port pitch clears a label — too fine a grid errors rather than overlapping labels.

The view authors each port's side and order directly in its `ports` block, and that listing is also the scope: a box shows exactly the ports it lists, and a wire draws only where both ends are listed. Place a port on the side facing the box it connects to.

### Connector pinouts

Pinout views document the physical connector face from the **harness side**. This is intentional: the drawing is meant to be used while building the harness. If the device datasheet gives a device-side view, mirror it before entering the layout.

Reusable connector types keep verbose physical metadata out of component definitions:

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
    pub port can_h "CAN H";
    pub port can_l "CAN L";

    connector x1: JstXh8p {
        pin 1: can_h;
        pin 2: can_l;
    }
}

view pinout "Controller pinouts" {
    grid: 20;
    include x1 at (0, 0);
}
```

A two-row connector can choose a numbering convention. `odd_even` fills columns first (`1,2` in the first column, `3,4` in the second), while `clockwise` and `counter_clockwise` walk the perimeter:

```
connector_type Mx150_16p "MX150 16p" {
    layout grid {
        rows: 2;
        cols: 8;
        numbering: odd_even;
    }
}
```

For sectioned connectors with mixed cavity sizes, use an explicit face layout. Coordinates are small grid slots, still harness-side; `size large` spans a 2x2 cavity.

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

```sh
wirebug render                       # discovers wirebug.toml by walking up from the CWD
wirebug render examples --out out/                  # one SVG per view, into out/
wirebug render examples --out out/ --pdf            # one A4 PDF, one view per page
wirebug render examples/wirebug.toml --out out/ --embed  # naked SVGs + manifest.json
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

To put the `wirebug` CLI on your `PATH` — which is also how the VSCode
extension finds its language server (`wirebug lsp`):

```sh
cargo install --path .
```

For the local development loop, install [`bacon`](https://dystroy.org/bacon/):

```sh
cargo install bacon --locked
```

Then run `bacon` (or `bacon serve`) to serve `examples/` with wirebug's live
reload while Bacon watches `src/` and restarts the app when the Rust code
changes. `bacon render` watches the app and example project sources, rendering
the example SVGs into `examples/out/` whenever either changes.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
