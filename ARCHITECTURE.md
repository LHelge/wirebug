# wirebug — architecture

How the application works: the pipeline, the data models, the renderers, the
checks, the source tree and the dependencies. `README.md` is the user-facing
intro; `AGENTS.md` holds the working rules (coding practices, commands,
releasing) and points here for everything about the application itself.

Keep this file true in the same change that alters what it describes.

## One pipeline

The only input is the **`.wb` DSL** (spec: `.claude/skills/wirebug-dsl/`).
Four CLI commands share it:

- **`check` (`src/dsl/`, then `src/render/lint.rs`)** — lex → parse →
  load project → resolve → elaborate → validate, then the **layout
  checks**: every view laid out by its renderer's own layout code, to find
  grids a renderer rejects, includes that collide and conductors no harness
  view draws (see DSL validation). Turns a multi-file `.wb`
  project into an elaborated IR (`ir::Design`) and reports problems via
  miette. One function sequences every stage after loading:
  `pipeline::check_loaded(&Project, load_problems) -> dsl::Checked`
  (the resolved registry, the design and all problems). The CLI, `serve`
  and tests reach it through `wirebug::check_project` (discover + load +
  `check_loaded`); the LSP loads through its overlay and calls
  `check_loaded` itself, keeping `Checked.resolved` for its completion
  index — so both drop a partial design and both get every new stage.
  `pipeline` lives outside `dsl` because the layout checks need `render`:
  `dsl` ends at `dsl::check_loaded` (resolve → elaborate → validate) and
  never depends on `render`, while `render::lint` depends on `dsl` like
  the rest of `render` does; `pipeline` is where both are visible.
- **`render` (`src/render/`)** — runs the same DSL pipeline, then draws
  every view in the resulting `ir::Design` to SVG (one file per view) plus
  an `index.html` (`render::index_html`) that embeds them all for browsing,
  grouped into **Schematics** / **Harnesses** tabs by view kind. Three view
  kinds render today: `schematic` (`render/schematic/`), `harness`
  (`render/harness/`), and `pinout` (`render/pinout/`). `--png` rasterises each view to PNG instead
  (`render/png.rs`, via `resvg`, at `png::PNG_SCALE` pixels per unit); `--pdf` writes one A4 PDF with one view
  per page (`render/pdf.rs`, via `svg2pdf` + `pdf-writer` — orientation per
  view by aspect ratio, named `<project-slug>.pdf`, replaces the per-view
  files and index; views render plain (`SvgMode::Plain`, no in-drawing
  title or stamp) and fixed-size page furniture carries them instead — a
  Helvetica-Bold title header, plus a Courier footer with the identity
  stamp and a `n / total` page number — so every page reads the same
  regardless of view scale); `--embed` emits host-styled "naked" SVGs
  plus a `manifest.json` sidecar in place of the HTML index (see below)
  and a `wirebug.css` starter stylesheet (`render::embed_stylesheet` —
  the built-in styles re-scoped under `.wirebug-{kind}`, referenced from
  the manifest's `stylesheet` field; hosts copy and theme it).
  Each view renders or fails on its own (see Render-time errors): `render`
  writes every view that rendered, prints each failure naming its view,
  and exits non-zero.
  The legacy YAML model/view loader has been removed; `ir::Design` is the
  only thing the renderer consumes.
- **`serve` (`src/serve/`)** — a live-reloading dev server. Renders the
  project into memory (the same `render_views` + `index_html` pipeline,
  `live_reload` on), serves it over axum, and watches the project tree for
  `.wb` and `wirebug.toml` changes; each save re-renders and pushes a
  websocket reload. Nothing hits disk. A failing `check` serves a diagnostics
  page that still live-reloads, so it recovers once fixed. A view that passes
  `check` but fails to render shows its error in its place on the index (the
  same template `render` writes), the other views stay visible, and the
  console logs each failure. `serve` is the
  only async command: `main` stays synchronous and spins a Tokio runtime just
  for this arm.
- **`lsp` (`src/lsp/`)** — a Language Server Protocol server over stdio,
  for editor integration (the VSCode extension in `editors/vscode/`).
  Synchronous (lsp-server's channel loop; stdout is the protocol, logs go
  to stderr). Per change it re-runs the whole check pipeline with open
  buffers shadowing the disk (`project::Overlay`), through the same
  `pipeline::check_loaded` as `check`, publishes per-file
  diagnostics (miette `Problem` → LSP via each problem's labels/severity/
  code, with explicit empty publishes so fixed files clear cross-file),
  and serves context-aware completion: an owned snapshot of `Resolved`
  (per-file scopes + merged components) consulted by a token-stack scan
  of the live buffer — lex-only, so completion survives parse breakage,
  and a file that fails to load, or loads without statements the parser
  skipped, keeps its last-good scope (`CompletionIndex::update_with`).
  A buffer with several broken statements gets a squiggle for each.
  Completion covers auto-import —
  `use ‸` offers every component/connector_type defined elsewhere,
  inserting the full `<Name> from "<relative path>";` remainder — plus
  `extend` targets, the `connector_type`/`layout` bodies, and the keys of
  a cable body and of a wire body (a `{` right after a wire's `]`). An open
  `.wb` file its project never loads (not reachable from `main.wb` via
  `use`) gets one info-severity `unlinked_file` notice instead of
  silence — rust-analyzer's unlinked-file move; its contents stay
  unchecked, but its top-level names are parsed into the index so
  auto-import can wire the fresh fragment in. Go-to-definition
  (`lsp/definition.rs`) re-runs load/resolve per request (never stale;
  projects are small) and span-walks the registry: use names/paths,
  instance type refs, connector type refs, wire endpoints, view
  includes/ports/enclosures, and `extend` → its root `component`, all
  merge-group aware. Hover/rename/semantic tokens/formatting are
  deliberately later.

The `index.html` is an [`askama`] compile-time template (`templates/`),
rendered by `render::index_html(views, manifest, live_reload)` — shared by
`render` (static, `false`) and `serve` (`true`, injects the reload script).
It takes `render_views`' per-view results, so a failed view keeps its
place in its kind's tab and shows its error instead of a drawing.

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
Renderers stamp `<name> v<version> · rev … (date) · wirebug v<appver>`
in the SVG's bottom-right corner — the trailing `wirebug v…`
(`stamp::APP_VERSION`, this crate's `CARGO_PKG_VERSION`) records the
generator and is always present; `stamp::stamp_text` is the single
source, so the three SVG renderers and the PDF footer all carry it. The
HTML index puts the project name in `<h1>` with the version and
description below it; PDF pages carry the same stamp text in their
fixed-size footer (and set the PDF `Producer` to `wirebug <appver>`).
Embed mode suppresses the in-drawing stamp, so `manifest.json` carries a
`generator: "wirebug v<appver>"` field instead.

`wirebug manifest version` prints the manifest's version `v`-prefixed
(`v0.1.0`) to stdout, for CI to check against a git tag. It's backed by
`dsl::load_manifest` (discovery + `manifest::load` only, skipping the
check pipeline), so it works even when the `.wb` sources don't compile.
Distinct from clap's `--version`, which prints the wirebug app version.

## DSL mental model

- **AST** — a faithful parse of one `.wb` file. `Definition` (a component
  *type*) holds `Port`/`Connector`/`Instance`/`Wire`/`Property`/nested-
  `Definition` members; `View`s are top-level siblings. A `Member::Property`
  is a line of the type's part data (`part: "…";`, read by
  `Definition::properties`): the parser reads a component-body statement
  that starts `<name>:` once (`named_member`) and the token after the colon
  decides — a name makes an instance (`part: Fuse;` is an instance named
  `part`), a string or number a property. A `Definition.kind` marks it
  `component` (introduces the type) or `extend` (a fragment merged into a
  same-named component). Every node carries a `Span`; type/instance/port
  references are *unresolved* `Spanned<Ident>`. A statement with a
  syntax error is skipped and leaves no node (see DSL validation).
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
  `None`. A conductor inside a `twisted { }` group also carries
  `Wire.twisted_group = Some(i)` (numbered per cable in source order), so
  grouping survives flattening. A wire's optional `{ key: value; }` body
  (`ast::Wire.properties`, the same `ast::Property` lines a cable body
  holds) elaborates to typed `Wire.length`/`r#type`/`part`; a conductor's
  `length` overrides its cable's, and `Instance::effective_length(wire)`
  answers the one that applies (its own, else its cable's
  `CableMeta.length`); `CableMeta.length` itself stays the cable's. A
  component type's part data elaborates to `Instance.part`, an
  `ir::PartMeta` of typed optional strings (`number` from `part:`,
  `manufacturer`, `description`, `rating`) collected across the type's
  fragments, root first, so every placement of the type carries the same;
  a synthetic inline instance's is empty (its halves' part data lives in
  `InlineMeta`). Every
  `Wire` keeps its authored `span` (the `wire` statement, inside its
  cable for a conductor), so a check over the drawings can point back at
  it. So the schematic renderer ignores cables
  for free, and only the harness renderer reads the tags. Cable
  conductors are 2-endpoint by rule; shared rails stay loose
  multi-endpoint wires.
- **Inline connectors (`inline`)** — a mated plug/receptacle pair
  mid-harness (a loom's branch point): one shared port set, two optional
  `male:`/`female:` housing halves, each a `connector_type` ref. Inlines
  share the instance namespace (endpoints say `ic.sig`) and elaborate as a
  **synthetic child `Instance`** marked by `Instance.inline: Option<
  InlineMeta>` (halves' description/`part`/layout) with one
  designator-named connector — so `rewrite_wire`, the harness layout's
  endpoint/node keys, and the schematic renderer need no special cases.
  A harness include selects a half (`include ic.female`); elaboration
  normalizes the include's connector slot to the designator and records
  `Include.half`, and the harness node renders that half's part identity
  plus an M/F header chip. Each loom's view shows the half crimped onto
  that loom; auto-scoping drops the other loom's conductors. Both halves
  in one view is a `duplicate_view_include`; schematic includes of
  inlines are `wrong_include_form`; `pub` on inline ports warns
  (`inline_pub_port`). Per-half pinout faces and a one-sided-pin lint are
  future work.

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
  verbatim), haloed and centred on the longest run clear of wire
  crossings and of codes already placed (the halo would visually cut a
  crossed wire; longest-segment midpoint is the fallback when nothing
  clears).
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
`MIN_PORT_PITCH / 2` (`Grid::of_view`); `check` rejects a finer one
(`grid_too_fine`). The keepout is why boxes need
room: a wired port needs two steps clear in front of it, and boxes closer
than two steps to each other close the corridor between them. `check`'s
Layout phase finds both, from the same `Placement` and router geometry.

### Harness views (`render/harness/`)

The dual of the schematic, WireViz-style — a **trunk-and-bezier** layout. An
include names a **connector** (`include <inst>.<connector> at (x,y)`), placed
at its authored centre; that connector becomes a **pin table** (header =
instance label + `<designator> · <description>` (bare designator when the
description is unset), one row per pin = number + label,
ordered by pin), **auto-scoped to the pins that carry a conductor in this
view** — a 60-cavity control connector shrinks to what the harness uses,
and an include with no conductors draws as a header-only box (visible, so
the author notices). The renderer derives a vertical **spine** at the x-midpoint of
the connectors (`layout.rs::spine_x`); each **pin** faces the connector its
conductor reaches (`face_pins` — East when that connector is to the right, West
when to the left, voting across the pin's conductors; an unwired pin or a tie
falls back to the spine side). So a node bridging both directions sends each
pin the short way rather than forcing the whole table to one side; `ConnectorNode.facing`
is kept as the node's dominant pin side. Connections are the *same* chain-decomposed subject wires, kept
only when both ends land on *included* connectors — a port's connector is found
via `ir::ConnectorRef.name`, so a connectorless / excluded / `Own` end drops
silently (like an unlisted port in a schematic). The candidates are the
subject's `layout::HarnessConductor`s (`HarnessConductor::all`: each chain
segment between connectors of two *different* children — one within a
single connector is a jumper, never drawn); `HarnessLayout::compute` keeps
those whose two connectors are included and records them as
`HarnessLayout.drawn` (`lint::Segment`, wire and pair index), which is what
`check`'s `undrawn_conductor` reads.

Kept conductors split by `Wire.cable`. A **declared cable** draws as a
**cable box** (`CableBox`/`render_cable_box`) on the spine: a titled table
(label + `type · length · ×count`), one coloured strand per row. A
`twisted { }` pair (exactly two conductors, grammar-enforced —
`ast::TwistedGroup` holds `[Wire; 2]`) draws its box run as a **symbolic
braid** (`draw::braid_d` — chained flex cubics swapping the pair's two
rows, an even half-twist count so each strand exits on its own row)
confined between the pair's annotation zones: the first row's label
anchors left, the second's right (`cable-label-start`/`-end` modifier
classes — a bare `text-anchor` attribute would lose to the
`.cable-label` stylesheet rule), each over the straight run where its
wire is unambiguously on its own row. Rows sort by endpoint-y with
groups kept adjacent (`layout::braid_partners` finds each strand's
partner, defensively straightening any non-pair group a hand-built
design might carry); the box is sized per-row, a pair needing label +
braid + label (`LABEL_CHAR_WIDTH` for the 9px font). Rows are
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
A two-tone color (`green/yellow`) adds a dashed tracer overlay on the
core and slash-joins its codes (`GN/YE`); `WireColor` carries
`base` + optional `tracer` (each an `ir::ColorName` — the closed IEC
60757 enum with an `Other(String)` escape hatch), split at the `/` by
the parser. Synonyms normalise on parse (`purple` → `violet`, also in
`data-color`); a non-IEC name warns in `check` but renders verbatim.

Three deliberate departures from the schematic's no-inference rule: pin
**facing** is derived per pin from where its conductor goes (above),
**pin-table scope** is derived from the view's conductors (above — an
explicit `ports { }` listing was considered and rejected for harnesses),
and wire routing is the bezier flex above — no object avoidance, unlike the
schematic's orthogonal router.
**Shields/drain wires are not drawn** (the IR carries no shield flag); reusing
the orthogonal router and adding shields are noted future refinements.

## DSL validation (`check`)

Problems are miette `Diagnostic`s (`dsl::diagnostics::Problem`), collected
so one run reports many. Errors fail the run; warnings fail only under
`--strict`. The checks, by phase:

- **Load** — file not found for a `use`; no `wirebug.toml`; IO.
- **Parse/lex** — syntax and lexical errors (with expected-token sets),
  and a reserved keyword written where a name belongs (`reserved_word`,
  on the word, with a rename hint). The parse takes the word as the name
  and goes on, so every occurrence of the word is reported, and the later
  phases check the name like any other (an `include wire` of an instance
  that does not exist is also an `unknown_include`).
  A syntax error does not stop the parse either: every `{ … }` block (a
  component body, a view, a connector type, a connector, an inline, a
  cable, a wire's body, a `ports { }` or `enclosure { }` block) and the
  file's top level recover per statement (`parse::block`). A statement that does not parse
  gets the error it got before recovery (same span, same expected set,
  the block's `}` included), is skipped up to its `;` or through the
  `{ … }` it opens (never past the `}` that closes its block), and the
  block reads on, so one run reports each broken statement once. The
  fixed-shape bodies (`twisted { }`, `layout grid { }`, `layout face { }`)
  stay one statement: a mistake inside skips the whole of it (a broken
  line in the body of a wire inside `twisted { }` is still skipped alone,
  since the wire body is a block of its own). Two guards
  against one mistake reporting twice: a statement that fails right after
  one that failed on its own `;` is that statement's tail, and is not
  reported, when it goes on where the `;` cut it — on the same line
  (`pub port ; a "A";`) or inside a `[`/`(` the cut statement left open (a
  `;` at a line end in a wire's endpoint list) — unless it starts with a
  reserved keyword (`parse::TokenLayout::continues`); on the next line,
  outside brackets, it is the next statement and reports its own error
  (`pub port a "A" pin;` then `l Lamp;` reports both); and a file whose braces do not balance (`parse::Completeness::
  Misnested`: every statement after the stray or missing `}` lands in
  another block) reports its first syntax error alone and is left out of
  the load, as if it had not parsed. The skipped statement's reserved
  words go with it: the parser rewinds out of the failed attempt, and the
  `TruncateState` notes rewind with it, so no note points into a skipped
  region and the notes of the statements around it all stand.
  A file with skipped statements (`Completeness::Holes`,
  `LoadedFile.holes`) loads without them and feeds the later phases, so
  its other definitions resolve for the files that use them. What a
  skipped statement declared is missing there, so while the project has a
  syntax error (`Project.parsed_whole` is false: a hole, a misnested file,
  or a lex error) the problems that report an absence
  (`Problem::reports_an_absence`: unknown or undefined names, unresolved
  or unused imports, no root, a view of nothing, an empty cable, a port
  without `pub` — a `;` too early parts `pub ; port a "A";` into a skipped
  `pub` and a private port) are dropped (`Project::drop_parse_cascades`,
  in `dsl::check_loaded`, so for the CLI and the LSP alike), and come
  back once the syntax is fixed. The problems that report something present and wrong (a bad
  gauge, a duplicate, an unknown color) are still reported: skipping a
  statement cannot cause them. Such a project yields no design
  (`check_loaded` drops it), so it never renders.
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
  As a warning: a view title already used by another view anywhere in the
  project (`duplicate_view_title`, compared exactly). The repeat is the
  primary label; the first view rides along as a miette `#[related]` note
  (`FirstViewTitle`), because one diagnostic carries one source file and the
  two views may sit in different files — the LSP turns a related note's
  labels into related information like any secondary label. Render's slug
  de-duplication (`overview_2.svg`) stays as the backstop.
- **Elaborate** — `main.wb` lacks a single top-level component (no root);
  containment cycle (a component instantiating itself transitively); a
  view of a component the design never instantiates
  (`uninstantiated_view_subject`, labelled on the view title,
  `ir::View.span` — say, a view in a library file `use`d only for its
  connector types). `ir::Design::subject_of` is the lookup both this check
  and `render` use to find the instance a view is drawn against.
- **Validate** — wire arity (fewer than two endpoints, error); cable wire
  arity (a cable conductor that isn't exactly two endpoints,
  `cable_wire_arity`); property checks on a cable body and on any wire's
  body and on a component's part data (`validate_properties`, keyed by
  `diagnostics::PropertyOwner`, whose `keys()` say what each body takes: a
  cable `type` and `length`, a wire `length`, `type` and `part`, a
  component `part`, `manufacturer`, `description` and `rating`, a closed
  set a later bill of materials reads) — an unknown key, a key set twice, a
  value of the wrong kind (strings for `type`/`part` and the component
  keys, a number for `length`). A component split with `extend` sets each
  key once across its fragments (`validate_fragment_properties`): a
  fragment may set a key the root left unset, and a key one fragment sets
  after another did is the same `duplicate_cable_property`, against the
  first setting (the root's first) — two labels when both fragments sit in
  one file, else `DuplicateFragmentProperty` with the first setting as a
  `#[related]` note (`FirstProperty`), as for `duplicate_view_title`.
  The codes keep the names from when only cables had properties
  (`unknown_cable_property`, `duplicate_cable_property`,
  `cable_property_type`); the messages name the owner (`unknown wire
  property`). Non-positive values (`invalid_gauge` on a wire or cable
  conductor's gauge, `invalid_length` on a cable's or a wire's `length`,
  both labelled on the number); an endpoint listed twice in one wire or cable
  conductor (`duplicate_endpoint`, labelled on both occurrences — compared
  as resolved `ir::WireEnd`s, so own port `gnd` and `ecu.gnd` never alias;
  two *different* wires on one port are a deliberate double crimp and are
  not compared). As warnings: a cable with no conductors (`empty_cable`),
  unused import, bare-port pin, and non-IEC 60757 wire color
  (`unknown_wire_color` — the `ir::ColorName::Other` escape hatch still
  renders verbatim). Property, arity and value checks and the endpoint
  check live here (not elaborate) so a type instantiated many times
  reports each once.

- **Layout** (`render/lint.rs`, run by `pipeline` after validate, and only
  when no earlier phase found an error — an erroring design never renders,
  and its views may hold includes that failed to resolve) — each view is
  laid out by its renderer (`SchematicRenderer::layout`,
  `HarnessRenderer::layout`, `PinoutRenderer::layout`, each returning a
  `lint::ViewLayout` built from the very layout `render` draws), so the
  check and the drawing cannot disagree. First the grid: each renderer
  validates its view's `grid:` with the function its `render` calls
  (`Grid::of_view` for a schematic, `GridFault::positive_step` for the
  others), and a rejected grid lands in `ViewLayout.grid` as a
  `render::GridFault` — the same value `render` turns into its
  `ViewErrorKind`. It is an error labelled on the grid's number
  (`ir::View.grid`, an `ir::ViewGrid` with the step and its span): a step
  that is not positive (`invalid_grid`, every view kind) or, for a
  schematic only, one under `MIN_PORT_PITCH / 2` (`grid_too_fine`, naming
  that minimum; harness and pinout views have none). A view on a rejected
  grid lays out nothing, so it reports the grid alone. Then errors, each
  labelled on the includes involved (`ir::Include.span`, the
  `inst`/`inst.connector` the include names; a view sits in one file, so
  plain two-label diagnostics):
  two includes whose boxes overlap (`overlapping_includes`, every view
  kind; boxes that only touch are fine, and the enclosure, text boxes and
  harness cable boxes are not includes); a wired schematic port whose
  routing stub (one clearance out) lands strictly inside another box's
  clearance-inflated keepout, so the visibility graph has no node there
  (`include_too_close`, one per pair of includes, naming every blocked
  port — `Router::blocked_stubs`, the router's own precondition); and a
  schematic wire whose two stubs the visibility graph does not connect —
  walled in by keepouts that meet (`unroutable_wire`, `Router::connects`,
  a breadth-first search over the same graph: A\* only weighs bends and
  crossings, never forbids an edge, so it fails exactly then). Overlapping
  pairs are not also reported as too close, and `unroutable_wire` is
  reported only in a view with neither. Together they are exactly the
  conditions under which `route_all` returns `RouteFailed`, so a project
  that passes `check` does not fail to route. Last, the canvas: each
  renderer's `layout` also reports `ViewLayout.canvas`, the viewBox of the
  view's standalone SVG (titled and stamped, the largest of the output
  modes and the one `--png` rasterises), from the very function its
  `render` sizes the viewBox with — a schematic routes every wire for it,
  since wires can detour outside the boxes, so it has a canvas only when
  nothing above stops routing. A canvas longer on a side than
  `render::MAX_CANVAS` (8192 units; the constant's comment says why) is
  `view_too_large`, an error in every view kind, labelled on the view's
  title (`ir::View.span`, stating the size) and on the includes whose
  boxes start first and end last along the longer side. `render::Canvas::fits`
  is the one comparison both this check and the rasteriser make, in `f32`
  as `usvg` reads the viewBox, so `check` rejects a view exactly when
  `--png` would refuse it. SVG and PDF output handle such a view (a
  2,000,000-unit view renders in milliseconds), but one rule for every
  mode keeps "passes `check`, renders" true whatever `render` is asked
  for. A later layout-derived check adds a field to `ViewLayout` and a
  mapping in `ViewLayout::problems`.
  As a warning, after every view's problems: a conductor of a component
  with at least one harness view that none of those views draws
  (`undrawn_conductor`, labelled on the conductor's `ir::Wire.span`, which
  may sit in another file than the views; the help names the includes that
  would draw it, as `lint::EndTarget`s — an inline connector only through
  the halves it declares, and one that declares none is told to declare a
  half first, since an include of an undeclared half is an error). Each harness view's `ViewLayout.drawn` is the
  `HarnessLayout.drawn` it renders; `lint::HarnessCoverage` unions them per
  subject type, and every `HarnessRenderer::conductors` entry (the same
  `HarnessConductor::all` candidates) outside the union is reported. So a
  multi-endpoint rail is judged per chain segment, an inline connector is
  covered through either half (an include of either normalizes to the
  designator), a double crimp is two conductors and never a problem, and
  a component with no harness view is not built from drawings yet, so is
  never reported. A harness view with a rejected `grid:` still counts its
  conductors (laid out on the default grid: which conductors a view draws
  does not depend on its grid), so the grid is reported alone rather than
  with a warning for every conductor the view draws.

Not done on purpose: **unconnected-port** detection. It needs per-instance
tree analysis and floods intentional unused-pin warnings on a real
component library — a separate, opt-in concern. See `dsl/validate/mod.rs`.

## Render-time errors

Reference and structural checks happen in `check` (the Resolve/Validate
phases above). Render adds only geometry/dispatch errors, in `src/error.rs`
(render-path only; DSL problems are miette `Diagnostic`s). Two levels:

- **`error::ViewError`** — one view failed: its `title`, its `view_kind`
  (so the index places the failure in the right tab), and a boxed
  `cause: ViewErrorKind`. It prints `view "<title>": <cause>`. Renderers
  and converters return `ViewResult<T>` (`Result<T, ViewErrorKind>`);
  `render_views`, the PNG step in `render::output` and
  `PdfExporter::to_pdf` (the last two via `RenderedView::failed`) attach
  the view, so no per-view
  error exists
  without the view it happened in. The kinds:
  - an unknown view `kind:` (`schematic`, `harness`, and `pinout` render
    today) — normally caught earlier by resolve's `unknown_view_kind`;
  - a schematic wire the router cannot route (`RouteFailed`, both ends as
    `ir::WireEnd` — `instance.port`, or a bare `port` for the subject's own
    port on the enclosure) — caught earlier by the Layout phase above, so a
    backstop for hand-built designs. The router itself only sees placed
    ports: `Router::route_all` fails with `route::Unrouted { pair }`, and
    the schematic renderer names that pair's wire ends
    (`layout::Connection::ends`);
  - a view subject type with no instance in the design — caught earlier
    by elaborate's `uninstantiated_view_subject`; a pinout include naming a
    connector its subject lacks — caught earlier by resolve's
    `unknown_connector`; both backstops for hand-built designs;
  - a non-positive `grid:`, or a schematic `grid:` finer than a port label
    needs (`NonPositiveGrid`, `GridTooSmall`, from the renderer's
    `GridFault`) — caught earlier by the Layout phase (`invalid_grid`,
    `grid_too_fine`), so a backstop for hand-built designs;
  - a canvas too large to rasterise (`CanvasTooLarge`, the size and
    `render::MAX_CANVAS`): `PngRasterizer::to_png` refuses a view whose
    viewBox exceeds the limit before allocating its pixmap — caught earlier
    by the Layout phase (`view_too_large`), so a backstop for hand-built
    designs. The pixmap itself is reserved fallibly (`try_reserve_exact`,
    then `Pixmap::from_vec`; `Pixmap::new` would abort the process when the
    allocator refuses), so a pixmap that still cannot be had, at a larger
    `ViewFormat::Png` scale, is a `PngEncode` failure of that view, not an
    abort;
  - our own SVG failing to parse or convert for `--png` / `--pdf`.
- **`error::Error`** — what stops the whole render: file IO when writing
  output (`Write`), the HTML index template (`Template`) and serializing
  the `--embed` manifest (`EmbedManifest`).

`render_views` returns one `Result<RenderedView, ViewError>` per view, in
declaration order; a failure never stops the others, and file names are
allocated to failed views too, so a failure never renames another view's
file. `render::output::Output::write` renders the design and writes it in
one mode (`main` maps the flags onto an `Output`), returning a `Written`:
the failures, and as its `Display` the summary line `render` prints. What
each output mode does with a failure:

- **SVG / `--png`** — the views that rendered (and rasterised) are written;
  `index.html` shows each failure, titled, where its drawing would be.
- **`--embed`** — the views that rendered are written, and `manifest.json`
  lists only those (its schema is unchanged: a host never sees a view whose
  file is missing). The failures are on stderr only.
- **`--pdf`** — `PdfExporter::to_pdf` takes `render_views`' results and
  makes one page per view that rendered and converted, numbered
  `n / total` among themselves; `PdfExport::failures` names the rest, in
  view order. `PdfExport::document()` is the only way to the bytes and
  returns `None` when every view of a design with views failed, so no
  PDF is written then.

In every mode `render` prints `error: view "<title>": <cause>` per failure,
reports `rendered <n> of <total> view(s)`, and exits non-zero. A file left
in `--out` by an earlier run of a now-failing view is not removed.

`render` runs `check_project` first and refuses to render a project that
has errors (or, under `--strict`, warnings).

## Source tree

```
src/
├── main.rs          # clap CLI: `check`, `render`, `serve`, `lsp`, `manifest version` (all over the .wb DSL)
├── lib.rs           # re-exports; check_project + render::render_views
├── pipeline.rs      # check_loaded: dsl::check_loaded, then render::lint
│                    #   (the layout checks) — shared by check_project and lsp
│
│  # ── DSL parse-and-check pipeline (the only input: .wb) ──
├── dsl/
│   ├── mod.rs           # check_loaded → Checked: resolve→elaborate→validate; CheckReport
│   ├── span.rs          # FileId, Span, Spanned<T>; Span→miette + chumsky::Span impl
│   ├── lex/
│   │   ├── mod.rs       # lex() → Vec<SpannedLexeme>; significant() = the trivia dial
│   │   └── token.rs     # Token, Trivia, Lexeme
│   ├── ast/mod.rs       # spanned AST; refs are unresolved Spanned<Ident>
│   ├── parse/mod.rs     # chumsky parser over &[(Token, Span)] → ast::File
│   ├── manifest/        # wirebug.toml loader + git-revision auto-fill
│   ├── project/mod.rs   # walk-up discovery + transitive `use` loading → Project
│   ├── resolve/         # DefId registry + reference checks, one pass per file
│   │   ├── mod.rs       # data model (Resolved, *Facts) + the resolve() driver
│   │   ├── register.rs  # pass 1: AST walk → DefInfo facts, local duplicates
│   │   ├── scopes.rs    # per-file type scopes (own defs + `use` imports)
│   │   ├── merge.rs     # MergeGroups: `extend` canonicals, orphans, dup guards
│   │   ├── refs.rs      # flat-namespace lookups + instance/connector/inline/
│   │   │                #   endpoint reference binding
│   │   └── views.rs     # view subject binding + include/enclosure checks
│   ├── elaborate/mod.rs # AST/registry → ir::Design; containment-cycle guard
│   ├── ir/mod.rs        # id newtypes + elaborated Design/Instance/Port/Wire/View
│   ├── validate/mod.rs  # wire arity, cable/wire properties, pin/gauge/length values (errors) + --strict warnings
│   └── diagnostics/mod.rs # miette `Problem` enum (one variant per failure class)
│
│  # ── ir::Design → SVG renderer ──
├── render/
│   ├── mod.rs       # render_views: subject lookup + per-view dispatch + slug,
│   │                #   one Result<RenderedView, ViewError> per view;
│   │                #   RenderedView{title,filename,kind,svg} + index_html (tabs)
│   │                #   + embed_manifest (manifest.json sidecar for --embed)
│   ├── geometry.rs  # Point, Bounds (+ overlap test); re-exports ir::Side
│   ├── lint.rs      # layout checks for `check`: overlaps, crowded ports,
│   │                #   unroutable wires, undrawn harness conductors,
│   │                #   from each renderer's `layout`
│   ├── output.rs    # Output::write: one render mode's files on disk (per-view
│   │                #   SVG/PNG + index or embed manifest, or the PDF) → Written
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
│   │   ├── layout.rs    # conductors (HarnessConductor), pin-table nodes,
│   │   │                #   spine + facing, cable boxes (centroid placement
│   │   │                #   + de-overlap), loose wires
│   │   ├── bezier.rs    # horizontally-flexed cubic bezier math (FLEX)
│   │   └── draw.rs      # SVG emission: pin tables, cable boxes, bezier wires
│   └── pinout/      # connector-face + pin-table renderer (kind: pinout)
│       └── mod.rs       # PinoutRenderer; cavity faces from connector layouts
│
│  # ── live-reloading dev server (`serve`) ──
├── serve/
│   ├── mod.rs        # serve(): discover root, build site, router, watcher, shutdown
│   ├── build.rs      # build_site: check, then Build::rendered (render→index_html);
│   │                 #   diagnostics page on error
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
│   ├── complete.rs   # CompletionIndex (owned Resolved snapshot, per-file
│   │                 #   last-good) + token-stack cursor contexts + auto-import
│   └── definition.rs # go-to-definition: fresh resolve per request, span walk
└── error.rs         # thiserror types (render path): ViewError per view, Error for IO/index
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
- Every major production (`port`, `wire`, `cable`, `view`, …) is
  **`.boxed()`** where it's defined. Chumsky combinators nest their
  sub-parsers' full generic types, so an unboxed embedding of a large
  sub-parser multiplies rustc's typeck cost geometrically — a bare
  `wire.then(wire)` once took builds from seconds to OOM. Boxing erases
  the type at each production for one dynamic dispatch per node. Keep
  new productions boxed, and never embed an unboxed heavyweight twice.
- Keywords are **reserved** or **contextual** (`Token::is_contextual`;
  one `KEYWORDS` table in `lex/token.rs` drives both lexing and display).
  Contextual ones — the view, `connector_type` layout and cavity words,
  plus `from` — mean something only in their own production; everywhere a
  name belongs they are names. Names go through two boxed productions,
  both taking an identifier or a contextual keyword (`Token::as_name`):
  `ident` for a name after a keyword, and `leading_name(separator)` for a
  name that starts a statement (`name: Type;`, `key: value;`,
  `side: ports;`, `port at (…);`). The reserved ones (the words that
  declare the design: `component`, `port`, `pin`, `wire`, `cable`, `view`,
  …) are taken as names too but recorded in the parser state, a chumsky
  `TruncateState` that forgets what a backtracked branch recorded;
  `parse_file` turns them into `ParseError::ReservedWord`. At a statement
  start a reserved word is a name only when the separator follows it;
  otherwise `leading_name` fails on the keyword without consuming it, so a
  misplaced keyword (`view` inside a component) keeps the error of the
  statements that could start there, on the keyword. A new keyword is
  reserved unless listed as contextual. LSP completion reads names through `Token::as_name` and
  only treats `ports {`/`enclosure {` as their blocks in their own
  position, so a connector named `ports` stays a connector body.
- Files are loaded once each (by canonical path), so a `use` cycle or a
  diamond import is harmless and never double-reports. Directory layout
  never affects logical hierarchy — only `use` paths and DSL nesting do.
- Wire endpoints are at most two-part (`inst.port` or bare `port`); the
  deep dotted form is an IR *path*, not surface syntax.

## Connector routing

Connector routing follows the orthogonal-routing paper (§4–6).
Nudging (§6) is implemented for the case our router produces — wires
sharing *collinear* channels (straight bundles): §6.1 orders a channel
by where each route enters it, and §6.2 spreads segments with a VPSC
solver that pins port ends and adds wall constraints keeping interior
segments outside the clearance-inflated boxes. Not implemented: the
paper's general branching-tree ordering (pseudo-direction + split
points) and alley-midpoint recentring. Revisit if a view needs them.

## Dependencies

What each crate is here for. Add new ones with `cargo add` so versions stay current.

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
- [`svg2pdf`] — vector SVG → PDF conversion for `render --pdf`
  (`render/pdf.rs`; brings its own `usvg`, a separate version from resvg's).
- [`pdf-writer`] — assembles the multi-page A4 document around `svg2pdf`'s
  chunks. Pinned to the version `svg2pdf`'s API exposes (its `to_chunk`
  returns this crate's types) — bump the two together.
- [`serde_json`] — serialises the `--embed` `manifest.json` sidecar.
- [`pathfinding`] — A* over the orthogonal visibility graph for
  object-avoiding connector routing (render path), and the breadth-first
  reachability test `check` uses to find wires the router cannot route.
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
[`svg2pdf`]: https://docs.rs/svg2pdf
[`pdf-writer`]: https://docs.rs/pdf-writer
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
