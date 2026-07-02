# wirebug — working notes

User-facing intro is in `README.md`. This file is for context that helps
future work on the codebase. Keep it short; don't restate the README.

## One pipeline

The only input is the **`.wb` DSL** (spec: `.claude/skills/wirebug-dsl/`).
Four CLI commands share it:

- **`check` (`src/dsl/`)** — lex → parse → load project → resolve →
  elaborate → validate. Turns a multi-file `.wb` project into an
  elaborated IR (`ir::Design`) and reports problems via miette.
- **`render` (`src/render/`)** — runs the same DSL pipeline, then draws
  every view in the resulting `ir::Design` to SVG (one file per view) plus
  an `index.html` (`render::index_html`) that embeds them all for browsing,
  grouped into **Schematics** / **Harnesses** tabs by view kind. Three view
  kinds render today: `schematic` (`render/schematic/`), `harness`
  (`render/harness/`), and `pinout` (`render/pinout/`). `--png` rasterises each view to PNG instead
  (`render/png.rs`, via `resvg`); `--embed` emits host-styled "naked" SVGs
  plus a `manifest.json` sidecar in place of the HTML index (see below).
  The legacy YAML model/view loader has been removed; `ir::Design` is the
  only thing the renderer consumes.
- **`serve` (`src/serve/`)** — a live-reloading dev server. Renders the
  project into memory (the same `render_views` + `index_html` pipeline,
  `live_reload` on), serves it over axum, and watches the project tree for
  `.wb` and `wirebug.toml` changes; each save re-renders and pushes a
  websocket reload. Nothing hits disk. A failing `check` serves a diagnostics
  page that still live-reloads, so it recovers once fixed. `serve` is the
  only async command: `main` stays synchronous and spins a Tokio runtime just
  for this arm.
- **`lsp` (`src/lsp/`)** — a Language Server Protocol server over stdio,
  for editor integration (the VSCode extension in `editors/vscode/`).
  Synchronous (lsp-server's channel loop; stdout is the protocol, logs go
  to stderr). Per change it re-runs the whole check pipeline with open
  buffers shadowing the disk (`project::Overlay`), publishes per-file
  diagnostics (miette `Problem` → LSP via each problem's labels/severity/
  code, with explicit empty publishes so fixed files clear cross-file),
  and serves context-aware completion: an owned snapshot of `Resolved`
  (per-file scopes + merged components) consulted by a token-stack scan
  of the live buffer — lex-only, so completion survives parse breakage,
  and a file that fails to load keeps its last-good scope
  (`CompletionIndex::update_with`). An open `.wb` file its project never
  loads (not reachable from `main.wb` via `use`) gets one info-severity
  `unlinked_file` notice instead of silence — rust-analyzer's
  unlinked-file move; its contents stay unchecked (lex/parse-only
  checking of orphans is a possible later step). Hover/goto-def/rename/
  semantic tokens/formatting are deliberately later.

The `index.html` is an [`askama`] compile-time template (`templates/`),
rendered by `render::index_html(views, live_reload)` — shared by `render`
(static, `false`) and `serve` (`true`, injects the reload script).

## Project manifest

Every project carries a `wirebug.toml` beside `main.wb` — a small TOML
manifest with a single `[project]` table. `name` and `version` are
required; `description`, `authors`, `license`, `revision`, and `date`
(parsed as `chrono::NaiveDate`) are optional. Unknown keys are a parse
error (`#[serde(deny_unknown_fields)]`).

The manifest is the project marker. The CLI deliberately follows Cargo's
shape here: from inside a project, `check`, `render`, and `serve` walk up
parent directories until they find `wirebug.toml`; commands may also point
directly at the project root or at the manifest. `main.wb` remains the
entry `.wb` file beside the manifest.

If `revision` is omitted, the loader fills it from git: the short HEAD
SHA, suffixed `-dirty` when the working tree has changes. An authored
`revision` always wins; outside a git repo (or without `git` on PATH)
the field stays `None`.

The parsed `Manifest` rides on `Project` → `ir::Design` (as an
`Option<Manifest>` so synthetic test designs need not invent one).
Renderers stamp `<name> v<version> · rev … (date)` in the SVG's
bottom-right corner; the HTML index puts the project name in `<h1>`
with the version and description below it.

## DSL mental model

- **AST** — a faithful parse of one `.wb` file. `Definition` (a component
  *type*) holds `Port`/`Connector`/`Instance`/`Wire`/nested-`Definition`
  members; `View`s are top-level siblings. A `Definition.kind` marks it
  `component` (introduces the type) or `extend` (a fragment merged into a
  same-named component). Every node carries a `Span`; type/instance/port
  references are *unresolved* `Spanned<Ident>`.
- **Resolved registry** — every definition (top-level and nested) keyed by
  `DefId`, with flattened ports (connectors are grouping metadata, not a
  namespace — port names are unique per component), per-file type scopes
  (own defs + `use` imports), and resolved instance/endpoint/include refs.
- **Merge groups (`extend`)** — a top-level component may be authored across
  files: `main.wb` has `component Vehicle { … }`, other files
  `extend Vehicle { … }`, and `main.wb` pulls each in with the usual
  `use Vehicle from "traction.wb";`. A same-name collision in a file's scope
  is a *merge* (not the `duplicate_type` error) as soon as either side is an
  `extend`. `resolve::MergeGroups` records the fragment set with one
  **canonical** id (the lone root `component`, kept first); `Resolved::
  fragments(d)` yields a component's fragments (just `[d]` when unmerged).
  Type resolution stays per-fragment (each carries its own `use` imports), but
  the merged component is one **flat namespace**: endpoints (`check_endpoint`),
  view includes/enclosures (`resolve/views.rs`), and elaboration all consult
  the union across fragments, so a wire or view in one file freely references
  an instance/port declared in another. `extend` is top-level only; a fragment
  with no root `component` is an `orphan_fragment`.
- **IR (`ir::Design`)** — the elaboration: a flat
  `IndexMap<InstancePath, Instance>` (hierarchical semantics, no recursive
  ownership; the tree lives in `children` links). One node per placement,
  addressed by a dotted path (`Vehicle.front.module_1.pack` — the root
  instance is named after its component type), with
  materialized ports and wires rewritten to `WireEnd::Own`/`Child`.
  Definitions vanish here; only concrete instances flow to the IR.
  A `cable` is flat too: its metadata lands in `Instance.cables`
  (`CableMeta`, keyed by designator) and each conductor stays a `Wire` in
  `Instance.wires` tagged with `Wire.cable = Some(name)`; loose wires are
  `None`. So the schematic renderer ignores cables for free, and only the
  harness renderer reads the tag. Cable conductors are 2-endpoint by
  rule; shared rails stay loose multi-endpoint wires.

## Render mental model (IR → SVG)

The renderer consumes `ir::Design` directly — there is no separate model.
`render::render_views` walks `design.views`; each view documents a
component *type* and is rendered against the first instance of that type
(the root for a top-level view). The subject instance's **direct children**
are the includable things; the subject's own **wires** are the connections.
`view.kind` dispatches: `schematic` (below), `harness` (after it), or
`pinout` (`render/pinout/`).

The DSL view authors each include's ports: `include <inst> at (x, y) ports {
<side>: <port>, ...; }`. That `ports` block is the single source of both
**layout** (side + order) and **scope** (which ports show). The renderer
adds no inference (`src/render/schematic/layout.rs`):

- **Sides** — read straight from the include's authored placements
  (`ir::Include.ports`, a `Vec<(PortName, Side)>`). Ports keep their authored
  order within a side. No vector-summing, no defaulting.
- **Visible ports** — a box shows exactly the ports it lists, in order; an
  include with no `ports` block is a bare box. (Listing is the subsetting
  mechanism — explicit, not derived from wires.)
- **Connections** — each subject wire (a multi-endpoint net) is
  **chain-decomposed** into consecutive pairs in the order written
  (`[a,b,c]` → `a–b, b–c`). A pair is drawn only when both ends resolve to a
  placed port: a *listed* `WireEnd::Child` port of an *included* instance, or
  a `WireEnd::Own` port *listed in the enclosure* (below). Excluded instances,
  unlisted ports, and own ports without an enclosure placement drop silently —
  a listed port whose wire lands on such an end shows as a bare stub.
  Each drawn segment carries a `wire-code` annotation — the wire's color
  as an IEC 60757 code (`ir::WireColor::code`; unknown names pass through
  verbatim), haloed and centred on the segment's longest run.
- **Enclosure** — an optional `enclosure { }` block draws the subject itself
  as a dashed box *wrapping* the schematic, with the subject's own ports on its
  boundary as **inverted** ports (facing inward, so an `Own` end's wire routes
  to the interior). Each port is placed `<port> at (x, y)` where one slot is a
  side keyword (`west`/`east` in x, `north`/`south` in y) pinning that axis to
  the edge, the other a grid coordinate along the free axis. The box
  auto-wraps the child boxes (`Grid::enclosure_inset` standoff); it is a
  routing endpoint set but never an obstacle (`Placement::component_bounds`
  excludes it). An inverted port labels like a normal port on the *opposite*
  side, so its name sits outside the boundary and its pin number inside.

Box geometry is unchanged from before, minus author-supplied sizes: a
view's `grid:` step (world units; `DEFAULT_GRID` when omitted), `include`
`x`/`y` is the box **centre** in **grid units** (renderer multiplies by the
step). Ports sit a fixed **two steps** apart, **centred** on each side
(even count straddles the centreline, odd puts the middle port on it). A
box is always an even number of steps, so its centre and every port land on
a grid line. The side margin (corner to first port) is a full pitch (two
steps). Boxes always size from the busiest side's port count — width also
respects a text minimum (`MIN_WIDTH`); there is no explicit `width`/`height`
in the DSL. The routing clearance (box keepout) is one grid step; the nudge
gap is the port pitch (two steps), so parallel wires in a channel space out
like the ports they fan from. Both keep bundles grid-integral, and routing
otherwise sees only world geometry. (Matching the keepout to the pitch was
tried and reverted: at two steps the inflated box rects overlap on tightly
packed views and close off the only corridor, so routing fails.)
Because the pitch is two steps, the grid must be at least
`MIN_PORT_PITCH / 2`; a finer grid errors.

### Harness views (`render/harness/`)

The dual of the schematic, WireViz-style — a **trunk-and-bezier** layout. An
include names a **connector** (`include <inst>.<connector> at (x,y)`), placed
at its authored centre; that whole connector becomes a **pin table** (header =
instance label + `<designator> · <part>`, one row per pin = number + label,
ordered by pin). The renderer derives a vertical **spine** at the x-midpoint of
the connectors (`layout.rs::spine_x`); each **pin** faces the connector its
conductor reaches (`face_pins` — East when that connector is to the right, West
when to the left, voting across the pin's conductors; an unwired pin or a tie
falls back to the spine side). So a node bridging both directions sends each
pin the short way rather than forcing the whole table to one side; `ConnectorNode.facing`
is kept as the node's dominant pin side. Connections are the *same* chain-decomposed subject wires, kept
only when both ends land on *included* connectors — a port's connector is found
via `ir::ConnectorRef.name`, so a connectorless / excluded / `Own` end drops
silently (like an unlisted port in a schematic).

Kept conductors split by `Wire.cable`. A **declared cable** draws as a
**cable box** (`CableBox`/`render_cable_box`) on the spine: a titled table
(label + `type · length · ×count`), one coloured strand per row. Rows are
ordered by each conductor's endpoint-y midpoint (the 1D occupancy step), the
box's vertical centre is its strands' centroid, and multiple boxes are pushed
apart along the spine (`build_cable_boxes`, gap `CABLE_GAP`). **Loose wires**
(`LooseWire`/`render_loose`) draw as a single bezier pin-to-pin, no box.

Every wire segment is one horizontally-flexed cubic bezier (`bezier.rs::flex`,
`FLEX = 0.4`): a cabled conductor is lead-in → straight box run → lead-out;
a loose wire is one curve. Control points share each endpoint's y and stay
within the endpoints' x-span, so the curve never overshoots its bounding box
(no viewbox padding needed). Each strand is stroked with `wire.color` (the SVG
`stroke`) over a wider black casing path (WireViz's trick, so light
colors read on any background) and annotated
`<label> · <gauge>mm² · <color code>` (IEC 60757 via `ir::WireColor::code`).

Two deliberate departures from the schematic's no-inference rule: pin
**facing** is derived per pin from where its conductor goes (above), and wire
routing is the bezier flex above — no object avoidance, unlike the schematic's
orthogonal router.
**Shields/drain wires are not drawn** (the IR carries no shield flag); reusing
the orthogonal router and adding shields are noted future refinements.

## DSL validation (`check`)

Problems are miette `Diagnostic`s (`dsl::diagnostics::Problem`), collected
so one run reports many. Errors fail the run; warnings fail only under
`--strict`. The checks, by phase:

- **Load** — file not found for a `use`; no `wirebug.toml`; IO.
- **Parse/lex** — syntax and lexical errors (with expected-token sets).
- **Resolve** — undefined type, unresolved import, duplicate
  type/instance/port, unknown instance/port in a wire endpoint,
  private-port access (a non-`pub` port referenced from outside), unknown
  view include, ambiguous view subject, duplicate connector designator
  (`duplicate_connector_name`), duplicate cable designator
  (`duplicate_cable_name`). `extend` fragments add: a fragment nested inside a
  component (`nested_extend`); an `extend` with no root `component` of that
  name (`orphan_fragment`); an instance/port name declared in two fragments of
  one merged component (the ordinary `duplicate_instance`/`duplicate_port`,
  reported once across the group). A cable's wire endpoints resolve exactly
  like a loose wire's. A view kind no renderer implements is
  `unknown_view_kind` (so `check` catches it; render's own guard is a
  backstop for hand-built designs). View `ports { }` placements get the
  same treatment as wire endpoints: unknown side (`unknown_port_side`),
  unknown/private port, and a duplicate-port-in-one-include guard
  (`duplicate_view_port`). Includes are checked per view kind: a `harness`
  include must name an existing connector (`unknown_connector`) and carry no
  `ports { }`; a `schematic` include must not name a connector — violations
  are `wrong_include_form`. A view's `enclosure { }` ports resolve against the
  *subject*: each anchor must name exactly one side in the slot for its axis
  (`enclosure_anchor`) and an existing `pub` subject port (`unknown_port` /
  `private_port`), with the same duplicate guard (`duplicate_view_port`).
- **Elaborate** — `main.wb` lacks a single top-level component (no root);
  containment cycle (a component instantiating itself transitively).
- **Validate** — wire arity (fewer than two endpoints, error); cable wire
  arity (a cable conductor that isn't exactly two endpoints,
  `cable_wire_arity`); cable property checks (`unknown_cable_property`,
  `duplicate_cable_property`, `cable_property_type` — `type` wants a string,
  `length` a number); unused import and bare-port pin (warnings). Cable
  property/arity checks live here (not elaborate) so a type instantiated
  many times reports each once.

Not done on purpose: **unconnected-port** detection. It needs per-instance
tree analysis and floods intentional unused-pin warnings on a real
component library — a separate, opt-in concern. See `dsl/validate/mod.rs`.

## Render-time errors

Reference and structural checks happen in `check` (the Resolve/Validate
phases above). Render adds only geometry/dispatch errors, in the slim
`error::Error` enum (`src/error.rs` — render-path only; DSL problems are
miette `Diagnostic`s):

- an unknown view `kind:` (`schematic`, `harness`, and `pinout` render
  today) — normally caught earlier by resolve's `unknown_view_kind`;
- a view subject type with no instance in the design;
- a non-positive `grid:`, or a `grid:` finer than a port label needs;
- file IO when writing the SVGs.

`render` runs `check_project` first and refuses to render a project that
has errors (or, under `--strict`, warnings).

## Out of scope for the MVP — resist drift

These land later, one at a time. Don't pre-bake hooks for them; we'll
redesign each when it lands.

- Graphviz/object-avoiding harness routing, BOM views, manifest emission
  (a basic SVG harness renderer has landed; see `render/harness/`)
- View composition / `extends`
- Theming, colour
- Auto-layout
- Non-rectangle component symbols
- Visual grouping of ports by connector on a side (bracket + label)
- Per-port styling (input/output, voltage class, gauge, etc.)
- Explicit `junction`/`splice` elements (shared rails stay loose
  multi-endpoint wires for now; `cable` conductors are point-to-point)
- Port re-export aliases (`pub port x "X" = inner.port;` — a pure boundary
  mapping with no wire). Decided against for now (2026-06): the wire-through
  pattern stays; revisit when a BOM view makes invented color/gauge on
  zero-length boundary wires actually hurt.

## Architecture

```
src/
├── main.rs          # clap CLI: `check`, `render`, `serve`, `lsp` (all over the .wb DSL)
├── lib.rs           # re-exports; dsl::check_project + render::render_views
│
│  # ── DSL parse-and-check pipeline (the only input: .wb) ──
├── dsl/
│   ├── mod.rs           # check_project: discover→load→resolve→elaborate→validate
│   ├── span.rs          # FileId, Span, Spanned<T>; Span→miette + chumsky::Span impl
│   ├── lex/
│   │   ├── mod.rs       # lex() → Vec<SpannedLexeme>; significant() = the trivia dial
│   │   └── token.rs     # Token, Trivia, Lexeme
│   ├── ast/mod.rs       # spanned AST; refs are unresolved Spanned<Ident>
│   ├── parse/mod.rs     # chumsky parser over &[(Token, Span)] → ast::File
│   ├── manifest/        # wirebug.toml loader + git-revision auto-fill
│   ├── project/mod.rs   # walk-up discovery + transitive `use` loading → Project
│   ├── resolve/mod.rs   # DefId registry, scopes, flattened ports, reference checks
│   ├── elaborate/mod.rs # AST/registry → ir::Design; containment-cycle guard
│   ├── ir/mod.rs        # id newtypes + elaborated Design/Instance/Port/Wire/View
│   ├── validate/mod.rs  # wire arity (error) + --strict warnings
│   └── diagnostics/mod.rs # miette `Problem` enum (one variant per failure class)
│
│  # ── ir::Design → SVG renderer ──
├── render/
│   ├── mod.rs       # render_views: subject lookup + per-view dispatch + slug;
│   │                #   RenderedView{title,filename,kind,svg} + index_html (tabs)
│   │                #   + embed_manifest (manifest.json sidecar for --embed)
│   ├── geometry.rs  # Point; re-exports ir::Side (sides are authored)
│   ├── png.rs       # SVG → PNG rasterisation for --png (resvg/usvg/tiny_skia)
│   ├── stamp.rs     # project-identity stamp text/element (corner of every SVG)
│   ├── schematic/   # rectangle-based SVG renderer (kind: schematic)
│   │   ├── mod.rs       # SchematicRenderer; render orchestration
│   │   ├── layout.rs    # Placement: derive sides + boxes/ports in world coords
│   │   ├── draw.rs      # SVG emission (named `draw`, not `svg`, to
│   │   │                #   avoid clashing with the `svg` crate)
│   │   └── route/       # orthogonal connector routing (paper §4–6)
│   │       ├── mod.rs       # Router: build OVG once, route_all + nudge
│   │       ├── geometry.rs  # Rect, Dir
│   │       ├── visibility.rs# orthogonal visibility graph (§4)
│   │       ├── astar.rs     # A* via the `pathfinding` crate (§5)
│   │       └── nudge/       # separate wires sharing a channel (§6)
│   │           ├── mod.rs       # pipeline: segments → order → place
│   │           ├── segments.rs  # maximal segments + shared-edge detection
│   │           ├── order.rs     # §6.1 order routes within a channel
│   │           ├── place.rs     # §6.2 final placement (two axis passes)
│   │           └── vpsc.rs      # separation-constraint solver
│   ├── harness/     # WireViz-style trunk-and-bezier renderer (kind: harness)
│   │   ├── mod.rs       # HarnessRenderer; render orchestration + STYLE
│   │   ├── layout.rs    # pin-table nodes, spine + facing, cable boxes
│   │   │                #   (centroid placement + de-overlap), loose wires
│   │   ├── bezier.rs    # horizontally-flexed cubic bezier math (FLEX)
│   │   └── draw.rs      # SVG emission: pin tables, cable boxes, bezier wires
│   └── pinout/      # connector-face + pin-table renderer (kind: pinout)
│       └── mod.rs       # PinoutRenderer; cavity faces from connector layouts
│
│  # ── live-reloading dev server (`serve`) ──
├── serve/
│   ├── mod.rs        # serve(): discover root, build site, router, watcher, shutdown
│   ├── build.rs      # build_site: check→render→index_html; diagnostics page on error
│   ├── state.rs      # AppState (RwLock<Site> + broadcast + Notify); Site = index + svgs
│   ├── server.rs     # axum router: GET / (index), /ws, fallback SVG-by-name
│   ├── livereload.rs # websocket handler broadcasting "reload"
│   └── watcher.rs    # notify watcher, 200ms debounce, .wb-only filter → rebuild+swap
│
│  # ── language server (`lsp`) ──
├── lsp/
│   ├── mod.rs        # run(): stdio Connection, capabilities, batched message loop
│   ├── state.rs      # ServerState: overlay, open docs, published set, index
│   ├── line_index.rs # byte offset ↔ LSP line / UTF-16 column
│   ├── uri.rs        # file path ↔ file:// URI (lsp-types 0.97 has no helpers)
│   ├── diagnostics.rs# check cycle → Problem → LSP diagnostics, publish/clear
│   └── complete.rs   # CompletionIndex (owned Resolved snapshot, per-file
│                     #   last-good) + token-stack cursor contexts
└── error.rs         # thiserror types (render path; incl. askama Template)
```

The VSCode extension lives at `editors/vscode/`: a TextMate grammar +
language config (declarative highlighting, no server needed) and a thin
`vscode-languageclient` in `src/extension.ts` that spawns `wirebug lsp`
(`wirebug.server.path` setting → newest repo `target/` build → PATH).
`npm run package` builds the installable `.vsix`; see its README.

The `serve` module renders into memory only; `templates/index.html` (askama)
is the shared index template for both `render` and `serve`.

DSL pipeline notes:

- The lexer recognises trivia (whitespace, comments) as first-class spanned
  lexemes; `lex::significant()` is the *dial* that drops them today — a
  future `fmt` swaps it for a trivia collector without touching the parser.
- `chumsky` parses a `(Token, Span)` slice; our `Span` implements
  `chumsky::span::Span` (context = `FileId`), so `e.span()` yields
  file-tagged spans directly. `Rich` errors become owned `ParseError`s.
- Files are loaded once each (by canonical path), so a `use` cycle or a
  diamond import is harmless and never double-reports. Directory layout
  never affects logical hierarchy — only `use` paths and DSL nesting do.
- Wire endpoints are at most two-part (`inst.port` or bare `port`); the
  deep dotted form is an IR *path*, not surface syntax.

## Coding practices

### Testing — lock in behavior

- Every public feature has unit tests alongside the code
  (`#[cfg(test)] mod tests`); the worked example renders end-to-end in
  `tests/`.
- Test names describe behavior, not implementation:
  `connection_to_missing_port_errors`, not `test_validate_returns_err`.
- Snapshot tests with [`insta`] for stable text output: AST `Debug`,
  CLI help, error messages, normalised structural views of an SVG. Review
  with `cargo insta review`.
- **Don't snapshot raw SVG strings** — layout pixels churn on every
  renderer tweak. Either assert on fragments (presence of expected
  `<rect>`s, port labels, wire endpoints) or snapshot a derived
  structural representation.
- CLI tests with [`assert_cmd`].
- A bug fix lands with the test that would have caught it.

### Type system — make illegal states unrepresentable

- Newtypes for "string with meaning" — `TypeName`, `InstanceName`,
  `PortName`, `Pin` in the DSL IR. Cheap, and the compiler stops you
  mixing them.
- Enums where two fields can't both be set, or where a value has a
  closed set of variants (`Side`, `WireEnd`).
- Typestate where it earns its keep, if the call sites benefit. Don't
  pre-bake it.
- Prefer std traits over bespoke helpers:
  `From` / `TryFrom`, `FromStr`, `Display`, `Debug`, `Ord` /
  `PartialOrd`, `IntoIterator`.
- `#[derive(...)]` over hand-written impls when possible.

### Idioms

- Behavior lives on types — methods and trait impls, not free
  functions. `pub fn foo(x: &Bar)` is a smell; make it
  `impl Bar { fn foo(&self) }`.
- `&str` over `String` in arguments unless ownership is required.
- Iterators over manual indexed loops where it reads more clearly.
- Visibility is private by default. `pub(crate)` for cross-module
  internal API. `pub` only when the crate boundary matters.
- `#[must_use]` on builders, validation outputs, and anything the
  caller shouldn't silently drop.
- No `.unwrap()` / `.expect()` outside tests.
- No `.clone()` to dodge a borrow-checker fight — fix the lifetime.
- `unsafe` is banned without an explicit, reviewed justification.

### Errors

- DSL problems are miette `Diagnostic`s (`dsl::diagnostics::Problem`) with
  source-tagged spans; the render path uses `thiserror` (`error::Error`).
  `anyhow` only in `main`.
- Library errors are concrete enums. Never `Box<dyn Error>`, never
  `Result<_, String>` — strings aren't errors.
- Each variant carries enough context to act on: a diagnostic carries the
  offending span and source; a render error names the view kind, subject,
  or grid value at fault.

### Dependencies

- Add with `cargo add <crate>` so Cargo writes the latest version into
  `Cargo.toml`. Don't hand-edit version strings.
- A new dependency gets a one-line justification in the commit
  message. Prefer std and small focused crates over kitchen-sink ones.

### Change discipline

- Small, focused commits — one logical change each.
- Read your own diff before committing.
- Before pushing: `cargo fmt && cargo clippy -- -D warnings && cargo test`.
- No dead code, no commented-out code. Git remembers.
- Comments explain *why*, not *what*. Names handle *what*. Only
  comment a non-obvious constraint, invariant, or workaround.
- Doc comments (`///`) on public items.

## Project-specific decisions

- Rust edition 2024. Stable toolchain. No MSRV pinning yet.
- Order-preserving maps (`indexmap::IndexMap`) throughout the DSL
  registry and IR (instances, ports, children). Source order is
  meaningful — as a render default and as a tie-breaker for diagnostics.
- SVG emission goes through the [`svg`] crate. It handles XML escaping
  of user-supplied labels (a small but real foot-gun if hand-rolled)
  and gives a discoverable element-builder API. We still own structure,
  classes, and embedded `<style>`.
- Connector routing follows the orthogonal-routing paper (§4–6).
  Nudging (§6) is implemented for the case our router produces — wires
  sharing *collinear* channels (straight bundles): §6.1 orders a channel
  by where each route enters it, and §6.2 spreads segments with a VPSC
  solver that pins port ends and adds wall constraints keeping interior
  segments outside the clearance-inflated boxes. Not implemented: the
  paper's general branching-tree ordering (pseudo-direction + split
  points) and alley-midpoint recentring. Revisit if a view needs them.

### Dependencies

Add with `cargo add` so versions stay current.

Runtime:

- [`chumsky`] — parser combinators (span-carrying `Rich` errors) for the
  `.wb` DSL. The lexer is hand-written; chumsky is confined to `dsl/parse/`.
- [`toml`] / [`serde`] (with `derive`) — parse `wirebug.toml` into the
  project manifest. Confined to `dsl/manifest/`.
- [`chrono`] (with `serde`) — `NaiveDate` for the manifest's optional
  `date` field; ISO `YYYY-MM-DD` parsing rides serde.
- [`miette`] (feature `fancy`) — `Diagnostic` derives plus the pretty
  terminal renderer for `check` (`--format json` uses `JSONReportHandler`).
- [`indexmap`] — order-preserving maps (DSL registry/IR).
- [`clap`] (derive) — CLI parsing.
- [`thiserror`] — typed library error enums; underpins the `Diagnostic`s.
- [`anyhow`] — error glue in `main` only.
- [`svg`] — SVG document emission with escaping handled (render path).
- [`resvg`] — SVG → PNG rasterisation for `render --png` (`render/png.rs`).
- [`serde_json`] — serialises the `--embed` `manifest.json` sidecar.
- [`pathfinding`] — A* over the orthogonal visibility graph for
  object-avoiding connector routing (render path).
- [`askama`] — compile-time HTML templates (`templates/`); the `index.html`
  shared by `render` and `serve`.
- [`axum`] (feature `ws`) — the `serve` HTTP server + live-reload websocket.
- [`tokio`] — async runtime, built only for `serve`'s command arm.
- [`tower-http`] (feature `set-header`) — `no-store` dev cache header.
- [`notify`] — filesystem watcher behind `serve`'s rebuild loop.
- [`tracing`] / [`tracing-subscriber`] (feature `env-filter`) — `serve` and
  `lsp` logs (the latter to stderr; stdout is the protocol).
- [`lsp-server`] — rust-analyzer's synchronous stdio transport for the
  `lsp` command; no async runtime needed (matching `serve`-only-async).
- [`lsp-types`] — the protocol types. 0.97 dropped `url::Url`, so the
  file-path↔URI round-trip is hand-rolled in `lsp/uri.rs`.

Dev / test:

- [`insta`] — snapshot tests.
- [`assert_cmd`] — black-box CLI tests.
- [`predicates`] — assertions for `assert_cmd`.
- [`tempfile`] — scratch project dirs for pipeline/render tests.

[`chumsky`]: https://docs.rs/chumsky
[`toml`]: https://docs.rs/toml
[`serde`]: https://docs.rs/serde
[`chrono`]: https://docs.rs/chrono
[`miette`]: https://docs.rs/miette
[`indexmap`]: https://docs.rs/indexmap
[`clap`]: https://docs.rs/clap
[`svg`]: https://docs.rs/svg
[`resvg`]: https://docs.rs/resvg
[`serde_json`]: https://docs.rs/serde_json
[`tempfile`]: https://docs.rs/tempfile
[`pathfinding`]: https://docs.rs/pathfinding
[`askama`]: https://docs.rs/askama
[`axum`]: https://docs.rs/axum
[`tokio`]: https://docs.rs/tokio
[`tower-http`]: https://docs.rs/tower-http
[`notify`]: https://docs.rs/notify
[`tracing`]: https://docs.rs/tracing
[`tracing-subscriber`]: https://docs.rs/tracing-subscriber
[`lsp-server`]: https://docs.rs/lsp-server
[`lsp-types`]: https://docs.rs/lsp-types
[`thiserror`]: https://docs.rs/thiserror
[`anyhow`]: https://docs.rs/anyhow
[`insta`]: https://docs.rs/insta
[`assert_cmd`]: https://docs.rs/assert_cmd
[`predicates`]: https://docs.rs/predicates

## Commands

```sh
cargo build
cargo test
cargo fmt
cargo clippy -- -D warnings

# check a .wb project
cargo run -- check examples               # or just `check` from inside the project
cargo run -- check examples/wirebug.toml
cargo run -- check --strict --format json examples

# render every view in a .wb project to SVG (one file per view, into --out)
cargo run --release -- render examples --out out/
cargo run --release -- render examples --out out/ --png    # rasterise to PNG
cargo run --release -- render examples --out out/ --embed  # naked SVGs + manifest.json

# serve a project with live reload (re-renders on every .wb save)
cargo run -- serve examples --port 3000   # then open http://localhost:3000

# the language server runs over stdio — editors spawn it, not humans
cargo run -- lsp

# build + install the VSCode extension (see editors/vscode/README.md)
cd editors/vscode && npm install && npm run package
code --install-extension wirebug-*.vsix
```

## Done definition for the MVP

- The render command above writes one valid SVG per view into `out/`.
- Opening an SVG shows something recognisable as a schematic: labelled
  rectangles with named ports on derived sides, pin numbers shown,
  Manhattan wires between connected ports.
- `cargo test` passes; the integration test renders the example project
  and asserts on SVG fragments (not a full snapshot).
