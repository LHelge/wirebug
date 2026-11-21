# wirebug

Text-defined electrical schematics and wiring harnesses.

**wirebug** describes an electrical system in a small text DSL (`.wb` files) and turns it into SVG schematics, wiring harness drawings, and connector pinout drawings. One model, many views — inspired by [Structurizr](https://structurizr.com/) / [LikeC4](https://likec4.dev/) for system architecture and [WireViz](https://github.com/WireViz/WireViz) for wiring harnesses.

A project is a directory rooted at a `wirebug.toml` manifest, with `main.wb` as the conventional entry file beside it. Components are *types* with `pub` ports; you instantiate them, wire the instances together, and split a system across files with `use`. The full language is documented in `.claude/skills/wirebug-dsl/` and mirrored in `.claude/skills/wirebug-dsl/`.

## Install

### Cargo

```sh
cargo install --locked wirebug
```

### Homebrew

```sh
brew install lhelge/tap/wirebug
```

### Prebuilt binary

Download the latest release binary for your platform (Linux x86_64, macOS x86_64/arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/LHelge/wirebug/main/install.sh | sh
```

This detects your OS and architecture, downloads the right binary from GitHub Releases, and installs it to `/usr/local/bin`. Set `WIREBUG_INSTALL_DIR` to change the install location. Falls back to `cargo install` if no pre-built binary is available.

### From source

```sh
cargo install --locked --path .
```

### VSCode extension

Each [release](https://github.com/LHelge/wirebug/releases/latest) attaches a
`wirebug-<version>.vsix`. Download it and install:

```sh
code --install-extension wirebug-<version>.vsix
```

Or, in VSCode: **Extensions** panel → **⋯** menu → **Install from VSIX…**. The
extension provides syntax highlighting plus live diagnostics and completion via
`wirebug lsp`, so it needs the `wirebug` binary on your `PATH` (any install
method above) or a path set in the `wirebug.server.path` setting.

## Why

When designing an EV conversion — or any electrical system that isn't a PCB — traditional EDA tools like KiCad are overkill, and iterating in a GUI gets tedious. WireViz solved this beautifully for harness drawings. wirebug brings the same text-defined, git-friendly, regenerable-from-source workflow to system-level schematics, and ties schematics to harness drawings via a shared model so the two stay consistent by construction.

## Status

Early and experimental. Expect breaking changes.

What works today:

- The `.wb` DSL front end, end to end via `wirebug check`:
  - a lexer and [`chumsky`](https://github.com/zesterer/chumsky) parser for the DSL;
  - multi-file projects — discover `wirebug.toml`, load `main.wb`, and follow `use` and `fragment` imports transitively;
  - name resolution (types, instances, ports, view includes);
  - elaboration of the type/instance hierarchy into a flat, addressable IR;
  - validation (undefined names, duplicates, private-port access, containment cycles, …);
  - layout checks that lay every view out as `render` would: includes whose boxes overlap, and schematic wires with no room to route, fail `check` instead of `render`;
  - rich diagnostics via [`miette`](https://github.com/zkat/miette) — source snippets, carets, `--format json`.
- Three SVG renderers driven straight off the elaborated IR, one file per view plus an HTML index that groups them into Schematics/Harnesses/Pinouts tabs:
  - a **schematic** renderer (rectangle blocks, labeled ports, object-avoiding orthogonal wire routing, each wire drawn in its colour);
  - a **harness** renderer (WireViz-style pin tables, a central spine, and bezier cable bundles);
  - a **pinout** renderer (connector cavity faces plus pin tables, authored from the harness side).
- `wirebug render` to disk (SVG, or `--png` rasterised, or `--pdf` for a single PDF with one view per page on A4, A3, Letter or Tabloid paper, or `--embed` for naked SVGs + a `manifest.json` sidecar).
- `wirebug export` — the build tables as CSV or JSON: the wire list (cut list), the connector pin tables (crimp sheet) and the bill of materials.
- `wirebug serve` — a live-reloading dev server that re-renders on `.wb` or `wirebug.toml` saves.
- A VSCode extension ([`editors/vscode/`](editors/vscode/)) — syntax highlighting plus live diagnostics and context-aware completion via `wirebug lsp`, a language server over stdio.

In transition / not yet:

- Object-avoiding harness routing (the harness renderer flexes beziers with no general obstacle avoidance; a loose wire that would pass behind a cable box runs in a lane of its own)
- A rendered BOM view (the BOM is an export table, not a drawing)
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

Print the project's version (from `wirebug.toml`), `v`-prefixed to match a
git tag — handy in CI to verify a pushed tag matches the manifest:

```sh
wirebug manifest version           # -> v0.1.0  (walks up to wirebug.toml)
wirebug manifest version examples  # or point at a project / manifest
```

It reads only the manifest, so it prints a version even when the `.wb` sources
don't yet check. (`wirebug --version` still prints the wirebug *application*
version.)

## Concepts

**Component** — a *type*, not an instance: a named definition with `pub` ports (its interface) and, optionally, instances of other components plus the wires between them (its implementation). Components can nest; nested definitions are private to their parent. A component may also say what it is bought as, with `part: "EV200AAANA"; manufacturer: "TE Connectivity"; description: "HV main contactor"; rating: "500 A";` lines in its body (all optional, each once, all strings); every placement of the type carries them. A `<name>:` followed by a type is an instance instead, so `part: Fuse;` places a `Fuse` named `part`.

**Port** — a named connection point with a human-readable label. `pub` exposes it to instantiators; visibility does not propagate automatically — a parent re-exports by declaring its own `pub` port and wiring it through.

**Connector** — physical grouping metadata (a required designator, an optional part description, and pin assignments). It is *not* a namespace: a port `c0` inside a connector is still referenced as `instance.c0`, and port names are unique across the whole component. Reusable `connector_type` definitions can carry shared metadata and pinout layout, then components instantiate them with the same port-declaration body as an inline connector: `connector x1: TypeName { pub port a "A" pin 1; }`.

**Instance** — a placement of a component type, with a name (used in wires) and an optional label (shown in diagrams).

**Wire** — a colour, a gauge (mm²), and two or more endpoints (`instance.port`, or a bare name: the enclosing component's own port, or one of its splices). Multi-endpoint wires model shared rails and T-junctions. A wire may end in a property body instead of its `;` — `wire red 0.5 "12V" [fuses.ecu, ecu.b_pos] { length: 1.2; type: "FLRY-B"; part: "..."; }` — giving its length, insulation type and part number (on a multi-endpoint rail, `length` is the whole rail's; give each leg its own length with a splice and one wire per leg). A length is in metres when bare, or takes a unit written directly after the number: `2m`, `15cm`, `150mm`. Point-to-point wires that travel together can be grouped into a `cable` with its own `type` and `length`; a conductor's own `length` overrides the cable's. A cable, or one of its `twisted { }` pairs, may be screened: `shield "SHLD" [ecu.shield_gnd];` names the pins its drain is terminated on — one (grounded at one end), two (both ends), or none (`shield;`, a screen that lands on no pin: cut back, or bonded at a gland or housing). A harness drawing marks a screened cable's box (a dashed outline, `shielded` in its subtitle) and a screened pair (a dashed outline round the pair), and draws each drain as a thin dashed line to the pins of the connectors it includes.

**Splice** — a joint inside a component that conductors end on: a crimp or solder splice in a loom. `splice sp_gnd "Ground splice" { part: "D-436-36"; }` declares it (label and body optional), and each conductor to it is an ordinary two-ended wire naming it like an own port: `wire black 0.5 [lamp.gnd, sp_gnd];`. All conductors on one splice are joined, each keeping its own colour, gauge and length — which a multi-endpoint rail cannot say. A cable's conductor may end on a splice, and a wire may join two splices; a wire to a splice has exactly two ends (`splice_wire_arity`), a splice fewer than two conductors end on warns (`splice_joins_nothing`), and its name may not repeat a port, instance, connector, cable or other splice of its component (`duplicate_splice_name`). A shared rail without a splice stays a loose multi-endpoint wire. A schematic draws no splice: the ports its conductors join show connected, like a rail.

**View** — a rendering target that documents a component: a kind (`schematic`, `harness`, or `pinout`), a grid, and what to place where. A `schematic` include lists the ports to show on each side; a `harness` include names a child instance's connector and draws its whole pin table; a `pinout` include names one of the subject component's own connectors and draws its cavity face. Wires are derived from the model, never listed in views — a wire draws only between ports/connectors both views include. In a schematic, a listed port whose wire goes somewhere the view does not show ends in a short stub naming where it goes (`→ vcm.can2_h`, `→ sp_gnd (lamp.gnd, +1 more)`, `→ out (leaves Vehicle)`, collapsing to `+N more` when the list is long), the full list in its tooltip. These references are fitted after the wires are routed and never move one: where the port's drawn wire runs on along the stub, the text sits beside it at the end of a short branch in the wire's colour, and where there is little room a reference says less (`→ 3 more`, the arrow alone, or just its stub, or nothing) rather than cover a wire or a label, sit nearer another port than its own, or grow the view past the canvas limit. A view documents its file's single top-level component unless it names one after its title — `view pinout "Sensor pinout" for Sensor { … }` — so a file defining several components gives each of its views a `for` (`view_subject` otherwise); `for` takes a component the file defines or imports. A view is drawn against an instance of the component it documents, so that component must be instantiated somewhere in the design (`uninstantiated_view_subject` otherwise). A schematic view may also carry free-text notes, label first like every other declaration: `text "120 Ω termination" at (8, 4);` centres a boxed label on that grid point. The older named form, `text <name> at (8, 4) "<label>";`, still renders but is deprecated and will be removed: `check` warns about it (`deprecated_text_syntax`) and shows the statement rewritten.

**Fragment** — a large component can be authored across files: one file introduces it with `component Vehicle { … }`, others add to it with `extend Vehicle { … }`, and the fragments merge into one component whose instances and ports every fragment's wires and views may reference. A file pulls a fragment file in with `fragment "systems/traction.wb";`: that loads the file without importing any name, so its `extend`s merge into the components of their names in scope and its views render. (`use Vehicle from "systems/traction.wb";` is the older spelling of the same and keeps working; prefer `fragment`, which says what it does.) An `extend` that merges into no component is an error (`orphan_fragment`); when the component exists but is not in scope where the file is pulled in, the help names the `use` to add and the file to add it to. `check` warns about a `fragment` line that adds nothing (`unused_fragment`, its help saying why): one that merges none of the file's `extend`s (because it has none, or their components are not in scope where the line is written) when the file adds nothing else either: no merging line of its own, and no view, or only views of a component its `extend`s never joined; one naming the entry file or its own file; and lines that only lead back to files already loaded, such as two files that name each other. It also warns about a file pulled in twice from one file (`duplicate_fragment`).

**Project** — a directory rooted at `wirebug.toml`, with `main.wb` beside it as the entry file. The CLI intentionally feels Cargo-like: from inside a project, commands walk up parent directories until they find the manifest; you can also point commands at the project root or at `wirebug.toml` explicitly. Logical hierarchy comes only from `use` and `fragment` imports and DSL nesting, never from directory layout.

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

`name` and `version` appear in the rendered output: as a small stamp in the bottom-right corner of every SVG, as the page header on the HTML index, and in every PDF page's footer beside the page number. `revision` and `date`, when set, are appended to the stamp. Every stamp also ends with the wirebug application version that produced the drawing (`· wirebug v<x.y.z>`), so a rendered artifact records which wirebug generated it.

**Revision from git.** If `revision` is omitted, wirebug shells out to `git rev-parse --short HEAD` in the project directory and uses the result, suffixed `-dirty` when the working tree has uncommitted changes. An explicit `revision = "..."` in the manifest always wins. If `git` isn't on `PATH`, or the directory isn't a git repo, the field stays empty and the stamp simply omits it — no error.

**Embedding into another document.** Pass `--embed` to `wirebug render` to emit SVGs intended for inclusion in another page, report, or static site. Embed-mode SVGs drop the built-in `<style>` block (the host stylesheet owns the look), suppress the bottom-right project-identity stamp, and class-tag the root `<svg>` with `wirebug wirebug-schematic` (or `wirebug-harness`) so host CSS can scope rules under `.wirebug`. The HTML index is replaced by a `manifest.json` sidecar listing every rendered view (title, filename, kind) plus the project's identity and a `generator` field (`wirebug v<x.y.z>`, since embed SVGs carry no in-drawing stamp), so a downstream build can enumerate and embed views without parsing each SVG. An embed SVG's root carries only its `viewBox`, no `width` or `height`, so your CSS sizes it (`width: 100%` fills the column); each view entry in the manifest also gives the view's natural size as `width` and `height` (numbers, after `title`, `filename` and `kind`), so a host that wants the drawing at the scale of standalone output — one unit to the CSS pixel, the same text size in every view — can set it from there. The build tables land beside it too, as `wires.json`, `connectors.json` and `bom.json` — the same documents `wirebug export … --format json` prints (see [Exporting build tables](#exporting-build-tables)) — and the manifest names them in its last field, `"exports": { "wires": "wires.json", "connectors": "connectors.json", "bom": "bom.json" }`; the fields before it are unchanged, so a host that ignores unknown fields is unaffected. A `wirebug.css` starter stylesheet lands beside the manifest (and is referenced by its `stylesheet` field): the built-in styles re-scoped under `.wirebug-schematic` / `.wirebug-harness` / `.wirebug-pinout`, so linking it reproduces the standalone look — copy it into your own assets and edit the copy to theme. Wires carry their colour on the element itself, as a `stroke` attribute plus `data-color` (`green/yellow`), so they show in colour with no rule for it, and a `stroke` your stylesheet sets on `.wire` or `.cable-wire` takes precedence. A copy of the starter stylesheet made before schematic wires were coloured has no rule for their casing and tracer and still sets `stroke: black` on `.wire`: it draws every wire black and thin, except that a two-tone wire shows a thin solid line in its tracer colour, so it no longer reads as either monochrome or coloured. To update such a copy, take the three `.wirebug-schematic .wire-casing`, `.wire` and `.wire-tracer` rules from the new `wirebug.css`. (The previous `--no-stamp` flag is removed: embedding was its only use case, and `--embed` now expresses it directly.)

## Rendering

`render` runs the same DSL pipeline as `check`, then draws every view in the design to its own SVG. Each SVG carries its natural size (`width` and `height`, one drawing unit to the CSS pixel), so a browser shows every view with text of the same size. The HTML index shows each view at that size too — also the `--png` index, whose images hold two pixels per unit for sharpness — shrinking only a view wider than the page, and never blows a small view up; click a drawing to zoom it to the window's width. Each `include` position is in **grid units**: the renderer multiplies by the grid step, and `x`/`y` are the box **centre**. Ports sit two grid steps apart and are centred on each side, so lining two components up makes the wire between them run straight. A box sizes itself from its busiest side (there is no explicit size in the DSL); omit `grid:` for the default. Each port's pin number sits outside the box; on a component with more than one connector it names the connector too, `hv:1` beside `intlk:1`, and a bare number means the component's only connector. On a north or south side, when any pin text would not fit level between two ports (a long designator, a ganged `1,2,3`, or a two-digit pin on a fine grid), that side's texts turn to run along their wires, reading toward the port so the number sits next to it; a pin number under the box's title moves out past the title. A `grid:` must be positive (`invalid_grid`), and a schematic's must be coarse enough that the two-step port pitch clears a label: `check` rejects a finer one (`grid_too_fine`, naming the minimum) rather than overlapping labels.

Boxes may touch but not overlap (`overlapping_includes`). A wire leaves its port by one grid step and keeps one grid step clear of every other box, so a wired port needs two grid steps of space in front of it (`include_too_close`), and boxes less than two steps apart close the gap between them — a wire walled in on every side is `unroutable_wire`. `check` lays each view out to find these, so a project that passes `check` does not fail to route in `render`.

A view may span at most 8192 units on a side, margins included (`view_too_large`, labelled on the view's title and the includes farthest apart). `--png` rasterises each view to one whole pixmap, so includes placed thousands of grid steps apart would ask for gigabytes of memory; at `--png`'s 2 pixels per unit the limit is 16384 px a side, more than an A0 sheet at 300 dpi and far beyond any real drawing. `check` holds every view to the limit whatever the output mode, so a project that passes `check` renders as SVG, PNG and PDF alike.

A harness view draws a conductor only when it includes what both of its ends land on: a connector (`include ecu.x1 at (0, 0);`), or one of the subject's splices by its name (`include sp_gnd at (20, 6);`). A splice draws as a solid dot labelled with its name, label and part, its conductors meeting at the dot — a cable's through its cable box, a loose wire as a loose wire. A loose wire (one in no cable) is drawn as one curve from pin to pin, annotated like a cable's conductor with its label, gauge and colour code (`WAKE · 0.5mm² · WH`); where that curve would pass behind a cable box on the central spine, where it would read as part of that cable, the wire runs through a lane of its own there instead, stacked with the boxes, its annotation cut short with an ellipsis if the lane is too short for it (the full text shows as a tooltip). Once a component has a harness view, `check` warns about each conductor between connectors of two of its children, or to one of its splices, that none of its harness views draws (`undrawn_conductor`, fatal under `--strict`), so a wire cannot go missing from every build drawing unnoticed. Each segment of a multi-endpoint rail counts on its own; an inline connector is covered through either half.

If a view still fails to render, `render` names it (`error: view "Overview": …`), writes every other view, and exits non-zero. The HTML index (and `serve`'s page) shows the error where that view's drawing would be; `--embed`'s `manifest.json` lists only the views written, and `--pdf` pages only those.

The view authors each port's side and order directly in its `ports` block, and that listing is also the scope: a box shows exactly the ports it lists, and a wire draws only where both ends are listed. Place a port on the side facing the box it connects to.

### Connector pinouts

Pinout views document the physical connector face from the **harness side**. This is intentional: the drawing is meant to be used while building the harness. If the device datasheet gives a device-side view, mirror it before entering the layout.

Each included connector is drawn as a table headed by the component and the connector (`Inverter · control`: the component's label when it is placed once, its type when it is placed more than once, since the drawing then serves every placement), with the housing's description and part number beneath. Its cavity face, when the connector type has a layout, sits in a panel of its own on top of the table, captioned *Viewed from harness side*; cavities no port uses are drawn dashed and greyed, and a label too long for its cavity is shrunk, or cut with an ellipsis (the full label is in the cavity's tooltip and in the table). Face and table are each as wide as their own content.

The table is a crimping sheet. It lists the pins in numeric order, whatever order they are declared in, then any port the connector gives no pin (marked *no pin*), and for each everything crimped into that cavity: a colour swatch with the IEC 60757 code (`GN/YE`, the tracer drawn as a stripe), the gauge (`0.5mm²`) and where the conductor goes (`→ Vehicle.vcm.wake (x1:3)`; for a splice, `→ Vehicle.sp_gnd (lamp.gnd, +2 more)`, its other ends listed as far as the column allows, the whole text in the tooltip; a conductor that leaves the component through one of its own ports on no connector, to be carried on by the parent's wire, reads `→ Vehicle.loom.out (leaves Loom)`). Two conductors in one cavity (a double crimp) are two lines under the one pin; a screen's drain is listed as *drain* with its cable, with no colour or gauge; a pin no conductor reaches shows a dash; a ganged port (`pins [46, 48]`) lists its conductors on each of its pins, marked *ganged 46+48*, so they are not counted twice. The conductors belong to a placement, so the table lists those of the component's first placement; when the type is placed more than once, the header says so: *Wires shown for Vehicle.front.bms, one of 2 placements*.

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
    connector x1: JstXh8p {
        pub port can_h "CAN H" pin 1;
        pub port can_l "CAN L" pin 2;
    }
}

view pinout "Controller pinouts" for Controller {
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
wirebug render examples --out out/ --pdf --paper a3 # the same on A3 (a4, a3, letter, tabloid)
wirebug render examples/wirebug.toml --out out/ --embed  # naked SVGs + manifest.json
```

`--pdf` lays each view on a page of the `--paper` size (A4 unless told otherwise), landscape for a view wider than tall and portrait otherwise, and scales it to fit inside the margins — down as far as it takes, up by at most 2×. A wide view fitted to a small sheet shrinks its text with it, so `render` warns of every page whose smallest text (a schematic's wire codes, a harness's cable labels, a pinout's shrunk cavity labels) would print under 5 pt. It names the smallest paper that prints it legibly, or, when none does, the paper that comes closest and the size it gives there, so a near miss reads differently from a view that needs redrawing:

```
warning: view "CAN bus": smallest text 4.3 pt on A4, below the 5 pt legibility minimum; legible on Tabloid (--paper tabloid)
warning: view "HV System Overview": smallest text 3.2 pt on A4, below the 5 pt legibility minimum; closest is Tabloid at 4.9 pt
```

The page is written all the same and the exit status is unchanged — also under `--strict`, which makes the project's own warnings fatal, not the paper's. `--paper` without `--pdf` is refused.

PNG and PDF output draw text in DejaVu Sans, which is built into wirebug, and in no other font: they use the same font on every machine and need no fonts installed. (Pixels may still differ slightly between CPU architectures.) A character DejaVu Sans has no glyph for — CJK, Thai, Devanagari and other scripts it does not cover — is drawn as an empty box in PNG and PDF. The SVG files name only `sans-serif` and do not carry the font, so a browser or embedding page draws them with its own sans-serif face, which can draw such characters. Harness and pinout tables are sized to hold their text as DejaVu Sans draws it, one of the widest sans-serif faces, so a title, a description line, a pin label or a cable's type stays inside its box in PNG and PDF, and in any browser whose sans-serif is no wider; with a narrower face a box shows a little more room around its text.

## Exporting build tables

`wirebug export` writes the tables a harness is cut, crimped and ordered from, built from the model rather than from any drawing — so they list every conductor, whether or not a view draws it:

```sh
wirebug export wires                                  # the wire list, as CSV on stdout
wirebug export connectors examples --out pins.csv     # the pin tables, into a file
wirebug export bom examples                           # the bill of materials
wirebug export wires examples --format json           # the same data as JSON, for tooling
```

Each table is a subcommand and takes the project like `check` and `render` do (a manifest, a project directory or a `.wb` file; by default the project around the current directory). `export` checks the project first and, like `render`, refuses one with errors (or, under `--strict`, warnings), exiting non-zero with nothing on stdout. Diagnostics always go to stderr, so stdout holds the table alone; with `--out <file>` the table goes to the file (its directory created if need be) and stderr gets a one-line summary (`exported 19 wire-list row(s) as csv to wires.csv`).

**CSV** (`--format csv`, the default) follows RFC 4180: a header row, one row per record, a field quoted only when it holds a comma, a quote or a line break (a quote doubled inside it), `\n` line endings, UTF-8 without a byte-order mark. Numbers use `.` as the decimal point whatever the locale; lengths are in metres and gauges in mm², as the column names say. An empty field means the value does not apply or is not set. A list inside one field (the pins of a ganged port, the ends beyond a splice) is separated by `;`. Twisted pairs are numbered from 1 within their cable, in source order. Instances are named by their full path (`Vehicle.front.lamp`) and ends by the full path of what they land on (`Vehicle.ecu.pwr`, `Vehicle.sp_gnd`).

Values are written as they are — no byte-order mark, no escaping of text that looks like a formula — so a table's CSV holds exactly what its JSON holds. Opened by double-click, Excel misreads three things: it decodes the text in the local codepage instead of UTF-8 (so `Ω`, `µ` and `å` come out garbled), it turns a part number such as `0430250400` into a number without its leading zeros, and it takes a label such as `+12V`, `=GND` or `-BATT` for a formula (`#NAME?`). Import the file instead, through **Data › From Text/CSV**, choosing UTF-8 (65001) as the file origin and the Text type for the columns that hold names, labels and part numbers.

An end, in either table, is eight columns after its prefix (`from`, `to`, `far`):

| Column | Holds |
|---|---|
| `from` | the full path of what the end lands on |
| `from_kind` | `port` (a port of a placed component, inline connectors included), `boundary` (the owner's own port, where the conductor leaves its component), `splice`, or `unresolved` (never in a project that checks) |
| `from_instance` | the instance the port is on, or the splice's owner |
| `from_connector` | the connector designator (an inline connector's name); empty for a connectorless port |
| `from_pins` | the pins the port occupies (`2;3` for a ganged port); empty when it has no pin |
| `from_port`, `from_port_label` | the port's name and label |
| `from_splice` | the splice's name |

**`wirebug export wires`** — one row per physical conductor (each segment of a multi-endpoint wire: a rail of four ends is three rows) and one per cable screen, in source order:

| Column | Holds |
|---|---|
| `kind` | `conductor` or `shield` |
| `owner` | the instance whose component body declares the wire |
| `cable`, `cable_label`, `cable_type`, `cable_length_m` | the cable it runs in, empty for a loose wire; repeated on each conductor and screen row of the cable, so summing `cable_length_m` counts a cable once per row; the BOM counts each cable once |
| `twisted_pair` | the twisted pair it is in (for a screen, the pair it screens) |
| `screens` | for a screen: `cable` (the overall screen) or `pair` |
| `label` | the wire's label (a screen's own label) |
| `color`, `color_code` | the colour name (`green/yellow`) and its IEC 60757 code (`GN/YE`) |
| `gauge_mm2` | the cross-section |
| `type`, `part` | the wire's insulation type and part number |
| `from…`, `to…` | its two ends, as above; for a screen, the pins its drain lands on (none, one or two) |
| `length_m` | the length this row adds to a total: its own (or its cable's), or a rail's whole length on the rail's first segment only; empty on a screen, which is cut with its cable |
| `rail_length_m`, `rail_segment` | on each segment of a wire with more than two ends: the whole rail's length, and the segment's place in it, from 1 |

A `length` on a wire with more than two ends is the length of the whole rail, so summing `length_m` gives the copper to cut, each rail counted once.

**`wirebug export connectors`** — one table per connector of each placed instance, and two per inline connector, its male and female half (a half it does not declare is a table too, with no `housing_type`, described by the inline's label, as the BOM lists it); within it, one row per conductor or drain crimped into each cavity (two for a double crimp), a cavity in pin order with nothing in it as one row of entry `empty` (so unused pins are listed), and then each port of the connector that has no pin assigned yet, with an empty `pin`; a connector with no cavity and no port at all is one `empty` row with no `pin` and no `port`, so every table the JSON lists is in the CSV too:

| Column | Holds |
|---|---|
| `instance`, `connector` | the instance and the connector designator |
| `half` | `male` or `female` on a half of an inline connector |
| `housing_type`, `housing_description`, `housing_part` | the connector type the housing is bought as (an inline half's own; empty for a half the inline does not declare, whose description is the inline's label) |
| `pin` | the cavity; empty for a port with no pin assigned |
| `ganged_with` | the other cavities of a ganged port (a ganged port's conductor is listed in each of its cavities) |
| `port`, `port_label` | the port on the pin |
| `entry` | `conductor`, `drain` (a cable screen's), or `empty` |
| `half_fit` | on an inline half: `assigned` when the conductor is known to be in this half, `undetermined` when it is not, so it is listed under both halves. The harness views decide: `assigned` when they place the conductor in this half alone (it is then not listed under the other), `undetermined` when they place it in neither or both. An inline is a mated pair, so this holds whether it declares one half, both or none: a half it does not declare cannot be included in a view, so a conductor is never `assigned` to it, and one no view places in the declared half is `undetermined` |
| `owner` | the instance whose body declares the conductor |
| `cable`, `twisted_pair`, `screens` | its cable and pair; for a drain, what the screen screens |
| `label`, `color`, `color_code`, `gauge_mm2` | the conductor's (a drain's: its screen's label only) |
| `far…` | the end across the conductor, as above (a drain's other end, when it is grounded at both) |
| `beyond` | when `far` is a splice: every other end the splice joins, through linked splices, as full paths |

**`wirebug export bom`** — the bill of materials: one row per thing to buy, with how many of it the design uses. Quantities are **per placement**: a component type placed six times counts six, and its connectors, cables and wires count six times over. Rows come by kind — housings, cables, loose wire, components, splices — and within a kind in the order the design first uses each. An item without a part number is never dropped: it is a row with `specified` `false`, counted like any other.

| `kind` | One row per | `quantity` counts | `specified` when |
|---|---|---|---|
| `housing` | connector-type part number (its `part:`); housings without one, per connector type; connector blocks that name no type, per description (an inline connector's half it does not declare, per the inline's label); with no description either, all in one row | housings: each connector of each placed instance, and **two** per inline connector, its male and female half — a half it does not declare is still a housing to buy, listed unspecified | it has a `part` |
| `cable` | cable `type`; cables without one, in one row | cables | it has a `type` (a cable body takes no `part`) |
| `wire` | loose wire — conductors in no cable — of one colour and gauge, and one `type` and one `part` where the wires set them, so two part numbers of one colour and gauge are two rows | conductors, each segment of a multi-endpoint wire one | it has a `part` |
| `component` | part number (`part:`); each type without one is a row of its own, its other part data still shown | placements | it has a `part` |
| `splice` | part (`part:`); splices without one, per label (their name when unlabelled) | splices | it has a `part` |

| Column | Holds |
|---|---|
| `kind` | `housing`, `cable`, `wire`, `component` or `splice` |
| `specified` | `true` when the row names what to buy, `false` when that is still to be chosen |
| `part` | the part number |
| `type` | a housing's connector type, a cable's or a wire's `type`, a component's type |
| `label` | on a splice row without a part: the label (or name) the row groups its splices by |
| `description`, `manufacturer`, `rating` | the part data: a housing's connector-type description (a connector block's own, or an undeclared inline half's inline label, when it names no type), a component's part data, a splice's description (the first item's, where items of one part number differ) |
| `color`, `color_code`, `gauge_mm2` | a wire row's colour and cross-section |
| `quantity` | how many, as above |
| `total_length_m` | on a cable or wire row: the summed length of those that have one — a cable row the cables' own `length`, not their conductors'; a wire row each rail's length once — empty when none has |
| `unmeasured` | on a cable or wire row: how many of `quantity` have no length, so are not in `total_length_m`; the total is complete only when this is `0` |
| `boundary` | on a wire row: how many of `quantity` end on the owning component's own port that is on no connector. Such a conductor continues a wire the enclosing component declares to that port (the wire-through pattern), so it is not a second piece of wire to order; it is still counted in `quantity`. A design whose components have connectors on their boundaries has none |
| `where` | the placements `quantity` counts, as full paths: `Vehicle.ecu.x1`, `Vehicle.ic.male` (an inline connector's half, declared or not), `Vehicle.can`, `Vehicle.front.lamp`, `Vehicle.sp_gnd`; empty on a wire row, whose conductors are the wire list's rows |

What it does not count: connector contacts, terminals and seals (the model does not hold them); a cable's screen, which is part of the cable; a cable's conductors as wire (they come with the cable; their own `type` and `part` are in the wire list). The root component is the design itself and is not a row, and neither are inline connectors (their halves are housings) nor **assemblies** without a part number — a component that places others and has no port of its own — whose parts are listed instead. An assembly **with** a part number is listed, and so are its parts: the BOM cannot know whether a numbered assembly is bought complete, so when it is, its parts are not to be ordered again. A component that places others but has ports of its own may be a part or a sub-assembly; the model cannot tell, so it is listed, and so are the wires in its body. A connector is one housing: the model holds one connector per mating, of the type it names.

**JSON** (`--format json`) carries the same data, structured: ends as objects with a `pins` array, a splice's `beyond` as an array of ends, the pin tables nested as `connectors`, each with its `housing`, its `cavities` (each with its `entries`) and its `unpinned` ports, and the bill of materials as `lines`, each with every column as a field (`null` where the CSV is empty, `specified` a boolean, `where` an array). Every document has the same top level:

```json
{
  "table": "wires",
  "format_version": 1,
  "project": { "name": "…", "version": "…", "revision": "…", "…": "…" },
  "generator": "wirebug v0.1.0",
  "rows": [ … ]
}
```

(`"connectors": [ … ]` for the pin tables, `"lines": [ … ]` for the bill of materials.) `format_version` is raised when a column or field changes name, place or meaning; new ones may be added without raising it.

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
cargo install --locked --path .
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

PNG and PDF output use the bundled [DejaVu Sans](https://dejavu-fonts.github.io/) font (version 2.37, in [`assets/fonts/dejavu/`](assets/fonts/dejavu/)), which is not under the licences above but under its own: the Bitstream Vera and Arev font licences, in [`assets/fonts/dejavu/LICENSE`](assets/fonts/dejavu/LICENSE), which ships with the fonts.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
