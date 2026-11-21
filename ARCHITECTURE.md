# wirebug — architecture

How the application works: the pipeline, the data models, the renderers, the
checks, the source tree and the dependencies. `README.md` is the user-facing
intro; `AGENTS.md` holds the working rules (coding practices, commands,
releasing) and points here for everything about the application itself.

Keep this file true in the same change that alters what it describes.

## One pipeline

The only input is the **`.wb` DSL** (spec: `.claude/skills/wirebug-dsl/`).
Five CLI commands share it:

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
  (`render/png.rs`, via `resvg`, at `png::PNG_SCALE` pixels per unit); `--pdf` writes one PDF with one view
  per page (`render/pdf.rs`, via `svg2pdf` + `pdf-writer` — on the
  `--paper` sheet (`a4`, the default, `a3`, `letter`, `tabloid`; refused
  without `--pdf`; the CLI's `PaperArg` maps into the library's
  `pdf::Paper`, which knows no flags), landscape for a view wider than
  tall and portrait otherwise (with equal margins, also the orientation
  with the larger scale), the view scaled to fit inside 15 mm margins,
  down without limit and up by at most 2× (`MAX_UPSCALE`, so a small view
  is never blown up to fill a large sheet), named `<project-slug>.pdf`,
  replaces the per-view
  files and index; views render plain (`SvgMode::Plain`, no in-drawing
  title or stamp) and fixed-size page furniture carries them instead — a
  Helvetica-Bold title header, plus a Courier footer with the identity
  stamp and a `n / total` page number — so every page reads the same
  regardless of view scale). A page whose view's smallest text prints
  below `pdf::MIN_LEGIBLE_PT` (5 pt) — the smallest text being
  `RenderedView::smallest_text`, the smallest px font in the kind's
  stylesheet (plus a pinout's cavity-label shrink floor; a test holds
  every built-in font declaration to one px size, so none escapes),
  times the page's scale — is kept and reported as a `pdf::SmallText`
  (`PdfExport::small_text`, then `Written::small_text`) whose
  `pdf::BestPaper` is the smallest paper, by area, that would print it
  legibly, or else the paper that comes closest and the size it gives;
  `main` prints each as `warning: view "<title>": …`, adding the
  `(--paper <name>)` hint for a legible paper, and leaves the exit status alone, also
  under `--strict` (the paper is a render option, not the project, so
  this is no `check` diagnostic). `--embed` emits host-styled "naked" SVGs
  plus a `manifest.json` sidecar in place of the HTML index (see below)
  and a `wirebug.css` starter stylesheet (`render::embed_stylesheet` —
  the built-in styles re-scoped under `.wirebug-{kind}`, referenced from
  the manifest's `stylesheet` field; hosts copy and theme it), and the
  build tables as `wires.json`, `connectors.json` and `bom.json` (see
  `export` below), which the manifest names in its last field, `exports`
  (`render::EmbedExports`) — added after the fields hosts already read,
  so they keep their names and places.
  Each view renders or fails on its own (see Render-time errors): `render`
  writes every view that rendered, prints each failure naming its view,
  and exits non-zero.
  The legacy YAML model/view loader has been removed; `ir::Design` is the
  only thing the renderer consumes.
- **`export` (`src/export/`)** — runs `check_project` like `render`,
  refuses a project with errors (or, under `--strict`, warnings) with the
  same message (`— not exporting`), then writes one build table:
  `export wires` (the wire list), `export connectors` (the pin tables)
  or `export bom` (the bill of materials), one subcommand per `export::Table` (`cli::ExportCommand`, each taking
  `cli::ExportArgs`: the target, `--format csv|json` — its own
  `ExportFormat`, not `check`'s `--format` — `--out <file>` and
  `--strict`). `export::Export::new(design, table, format)` builds the
  text; `main` prints it to stdout, or `Export::write` saves it (creating
  its directory, as `render` creates `--out`) and `main`
  prints the returned `Exported` (its `Display` is the one-line summary)
  to stderr. Diagnostics go to stderr only, so stdout is the table alone.
  See Build tables for the wire format.
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
  inserting the full `<Name> from "<relative path>";` remainder; `fragment
  ‸` offers every other file holding a top-level `extend` or a view
  (`ast::File::is_fragment_target`), inserting `"<relative path>";`, and
  nothing once the cursor is inside a string (`Cursor.in_string`) — plus
  `extend` targets, the `connector_type`/`layout` bodies, and the keys of
  a cable body, of a wire body (a `{` right after a wire's `]`) and of a
  component's part data (beside the member keywords), the length units
  on a number the cursor touches after a key that takes a length
  (`length: 150‸`; each item's text edit replaces the unit typed so far,
  `Cursor.unit`), and `for` after a
  view's title with the file's component types after it; inside a view,
  includes, `ports { }` and `enclosure { }` complete against the
  component its `for` names (`FileScope.top_level`, then
  `FileScope.imported`), else the file's lone top-level one. A
  component's splices complete where a bare endpoint goes (not in a
  `shield`'s list) and as a harness include, `splice` is a member keyword,
  and a splice body (`Block::Splice`, opened only where `splice` starts
  the statement) offers its keys. An open
  `.wb` file its project never loads (not reachable from `main.wb` via
  `use` or `fragment`) gets one info-severity `unlinked_file` notice
  instead of silence — rust-analyzer's unlinked-file move; its contents
  stay unchecked, but its top-level names are parsed into the index so
  auto-import (and `fragment ‸`) can wire the fresh fragment in.
  Go-to-definition (`lsp/definition.rs`) re-runs load/resolve per
  request (never stale; projects are small) and span-walks the registry:
  `use` names, `use` and `fragment` paths (to the top of the file),
  instance type refs, connector type refs, wire endpoints (a bare name to
  its splice, through `Resolved::splice`, else its own port), a view's
  `for` subject, view includes/ports/enclosures (a harness include of a
  splice to the splice), and `extend` → its root
  `component`, all
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

- **AST** — a faithful parse of one `.wb` file. A `File` holds its `use`
  imports (`File.uses`), its `fragment` lines (`File.fragments`, an
  `ast::Fragment` being just the path) and its items; `File::imports()`
  lists the `use` and `fragment` lines together in source order, as
  `ast::Import`s, which is the order the loader follows them in. A path
  names a file through `Project::import_target` (relative to the naming
  file's directory, canonicalized), the one lookup resolve and the LSP
  share. `Definition` (a component
  *type*) holds `Port`/`Connector`/`Instance`/`Wire`/`Property`/nested-
  `Definition` members; `View`s are top-level siblings, each with an
  optional `subject` (the component named by `for` after its title;
  `None` leaves the subject implicit, see the registry). A `Member::Property`
  is a line of the type's part data (`part: "…";`, read by
  `Definition::properties`): the parser reads a component-body statement
  that starts `<name>:` once (`named_member`) and the token after the colon
  decides — a name makes an instance (`part: Fuse;` is an instance named
  `part`), a string or number a property. A `Definition.kind` marks it
  `component` (introduces the type) or `extend` (a fragment merged into a
  same-named component). Every node carries a `Span`; type/instance/port
  references are *unresolved* `Spanned<Ident>`. A view's `text` boxes
  (`ast::TextBox`, schematic annotations) are written label first, `text
  "<label>" at (x, y);`, and have no name; the deprecated named form,
  `text <name> at (x, y) "<label>";`, parses into the same node with
  `name: Some(_)` (one boxed `text_box` production, the token after `text`
  choosing the form) and flows through to `ir::TextBox.name`, which the
  schematic renderer draws as the box's `data-text` — so a host styling an
  old box keeps working until it is migrated; a nameless box has none. A
  statement with a syntax error is skipped and leaves no node (see DSL
  validation). A cable's screen is an `ast::Shield` (optional label, the
  `Endpoint`s its drain lands on, kept faithful in number): the cable body's
  own `shield` lines are `Cable.shields`, a pair's are
  `TwistedGroup.shields`; `Cable::wires()` yields conductors only, and
  `Cable::endpoints()` every reference the cable names, conductors' and
  shields' alike, which is what resolve (`check_endpoint`) and the LSP's
  go-to-definition walk; `Cable::shield_ends()` the shields' alone, which
  resolve checks may not name a splice. A `splice` member is an
  `ast::Splice` (name, optional label, the `ast::Property` lines of an
  optional body; `Definition::splices`).
- **Resolved registry** — every definition (top-level and nested) keyed by
  `DefId`, with flattened ports (connectors are grouping metadata, not a
  namespace — port names are unique per component), per-file type scopes
  (own defs + `use` imports; a `fragment` adds no name), and resolved
  instance/endpoint/include refs. A definition's splices
  (`DefInfo.splices`) share the own ports' namespace: a bare endpoint
  names one or the other, and `Resolved::wire_end(d, ep)` is the one
  mapping from an authored endpoint to an `ir::WireEnd` (a bare name a
  splice of the merged component declares is `WireEnd::Splice`, else
  `Own`), which elaboration and validation both call, so they agree on
  which references are one end; resolve's `check_endpoint` makes the same
  lookup (`Resolver::lookup_splice`).
  Each view is bound to its subject (`ViewBinding.subject`, in
  `resolve/views.rs`): the component its `for` names, looked up in the
  view's file scope (own top-level components and `use` imports, not
  nested ones), else the file's single top-level component; either way
  canonicalized, so a view in an `extend` fragment documents the merged
  component. Everything downstream keys on that subject (by type name in
  `ir::View.subject`), never on the file, so one file may hold views of
  several components.
- **Merge groups (`extend`)** — a top-level component may be authored across
  files: `main.wb` has `component Vehicle { … }`, other files
  `extend Vehicle { … }`, and `main.wb` pulls each in with
  `fragment "traction.wb";`, or the older `use Vehicle from "traction.wb";`,
  which keeps working unchanged. A same-name collision in a file's scope
  is a *merge* (not the `duplicate_type` error) as soon as either side is an
  `extend`; that is how the `use` spelling merges. A `fragment` brings no
  name into the scope: once the file's scope is complete (own definitions,
  then every `use`, so the lines' order does not matter), each top-level
  `extend` of the named file merges into the group of the scope entry of
  its name (`Resolver::merge_fragment`). `build_scopes` makes the merges of
  a file's `use` and `fragment` lines last, sorted by line, so a group
  gathers its fragments in source order whichever spelling names them (the
  order instances elaborate in, and which of two duplicates is reported). An `extend` whose
  component is not in the naming file's scope is not merged by the line, so
  a file of nothing but `fragment` lines passes nothing on to a file that
  names it: merging is not transitive, as with `use`.
  `resolve::MergeGroups` records the fragment set with one **canonical**
  id (the lone root `component`, kept first); `Resolved::fragments(d)`
  yields a component's fragments (just `[d]` when unmerged).
  Type resolution stays per-fragment (each carries its own `use` imports), but
  the merged component is one **flat namespace**: endpoints (`check_endpoint`),
  view includes/enclosures (`resolve/views.rs`), splices, and elaboration all consult
  the union across fragments, so a wire or view in one file freely references
  an instance/port declared in another. `extend` is top-level only; a fragment
  with no root `component` is an `orphan_fragment` (`Resolver::is_orphan`),
  and its wires and the includes of views documenting it are not checked:
  they reach members of the component it never joined, so they would only
  echo the error.
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
  grouping survives flattening, and `CableMeta.twisted[i]` (an
  `ir::TwistedMeta`, `CableMeta::twisted_group(i)`) is that pair's place
  for data of its own. Screens are typed data, never a fake `Wire`: the
  cable's overall one is `CableMeta.shield` and a pair's
  `TwistedMeta.shield`, each an `ir::Shield` (label, span and a
  `ShieldTermination` — `NoPin` (cut back, or bonded at a gland or
  housing: the model does not say which), `Single(WireEnd)` or
  `Both([WireEnd; 2])`, so more than two ends cannot be represented;
  `Shield::ends()` lists them). `CableMeta::shields()` yields every screen
  with what it screens (`ir::Screened::Cable` or `Pair(i)`), which is how
  a wire list asks whether a cable or a pair is shielded and where its
  drain lands. Elaboration keeps the first shield of a body and the first
  two ends of a shield; validate reports the rest. A wire's optional `{ key: value; }` body
  (`ast::Wire.properties`, the same `ast::Property` lines a cable body
  holds) elaborates to typed `Wire.length`/`r#type`/`part`; a conductor's
  `length` overrides its cable's, and `Instance::effective_length(wire)`
  answers the one that applies (its own, else its cable's
  `CableMeta.length`); `CableMeta.length` itself stays the cable's. A
  length is an `ir::Length`, whole micrometres (`u64`, always at least
  one), built by `Length::new(value, LengthUnit)` from the AST's
  `ast::Quantity` (the number and the unit written after it, `150mm`;
  `Quantity::length`, a bare number in metres), so lengths in any unit
  compare (`Ord`), add (`+`) and print exactly: `Length::metres()` normalises,
  `Display` prints metres with only the decimals the length has
  (`0.15 m`). The closed `LengthUnit` set (`m`, `cm`, `mm`) is
  `LengthUnit::ALL`. A length that does not convert (an unknown unit, not
  positive) elaborates to `None`; validate reports it. A
  component type's part data elaborates to `Instance.part`, an
  `ir::PartMeta` of typed optional strings (`number` from `part:`,
  `manufacturer`, `description`, `rating`) collected across the type's
  fragments, root first, so every placement of the type carries the same;
  a synthetic inline instance's is empty (its halves' part data lives in
  `InlineMeta`). A component's splices elaborate to `Instance.splices`
  (`ir::Splice`: `SpliceName`, label, `part`, `description`, span),
  gathered across its fragments; a conductor to one has a
  `WireEnd::Splice` end, and each conductor stays a two-ended `Wire` of its
  own. The IR answers what a splice joins:
  `Instance::splice_conductors(name)` (each conductor, a wire list's row
  with the splice as an end; `Wire::other_end` gives the pin across it),
  `Instance::splice_net(name)` (an `ir::SpliceNet`: the splices linked to
  it by splice-to-splice conductors and every other end of their
  conductors, as `NetEnd`s with their wire) and `Instance::splice_nets()`
  (one per group of linked splices). Every
  `Wire` keeps its authored `span` (the `wire` statement, inside its
  cable for a conductor), so a check over the drawings can point back at
  it. So the schematic renderer ignores cables
  for free, and only the harness renderer reads the tags. Cable
  conductors are 2-endpoint by rule, and so is any wire to a splice;
  shared rails without a splice stay loose multi-endpoint wires. A rail
  is one physical conductor per pair of consecutive ends, in the order
  written (`[a, b, c]` → `a–b`, `b–c`): `Wire::segments()` yields those
  runs (`ir::WireSegment`: its `index` along the wire and its two ends),
  and it is the only place the decomposition is written — the schematic
  layout, the harness layout (`HarnessConductor::all`) and the wire list
  (`export::WireList`) all take their segments from it.
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
  (`inline_pub_port`). The pin-table export reads which half a conductor
  is crimped into off the harness views (see Build tables). Per-half
  pinout faces and a one-sided-pin lint are future work.

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
  (`[a,b,c]` → `a–b, b–c`), by `ir::Wire::segments` (see the IR). A pair
  is drawn only when both ends resolve to a placed port: a *listed* `WireEnd::Child` port of an *included* instance, or
  a `WireEnd::Own` port *listed in the enclosure* (below). Excluded instances,
  unlisted ports, and own ports without an enclosure placement draw no wire;
  a listed port whose wire lands on such an end gets an **off-view
  reference** instead (below). Being unplaced is the only reason a
  segment is not drawn.
  A splice has no box and no port in a schematic (no schematic syntax
  names one): each `Instance::splice_nets()` net is drawn as what it is
  electrically, one rail between the far ends of its conductors — those
  the view places, in wire order, each port once — chain-decomposed like
  a multi-endpoint wire, each segment in the color of the conductor that
  brings its second end in. No junction dot, as for any rail. The segments
  join `Placement`'s connections, so the router and the Layout lints
  (`include_too_close`, `unroutable_wire`) see exactly what is drawn.
  Each drawn segment carries a `wire-code` annotation — the wire's color
  as an IEC 60757 code (`ir::WireColor::code`; unknown names pass through
  verbatim), haloed and centred on the longest run clear of wire
  crossings and of codes already placed (the halo would visually cut a
  crossed wire; longest-segment midpoint is the fallback when nothing
  clears). Each segment is stroked in its color the way the harness
  strokes a strand (see Wire strokes below): a `wire-casing` polyline, the
  `wire` core in the base color, and a `wire-tracer` for a two-tone wire —
  so a splice rail shows which conductor brings each port in. Wires are
  drawn **one net at a time** (`Connection::nets`: connections joined,
  directly or through each other, by a shared placed port — the segments
  of a multi-endpoint wire or of a splice net, and wires crimped into one
  port): all of a net's casings, then its cores, its tracers and its codes,
  members in source order, nets in the order of their first connection
  (an order-preserving union-find, nothing hash-ordered). Connections
  sharing a port leave it through the same stub, so drawing wire by wire
  would lay a later casing over an earlier core where they split and cut
  the joint like a crossing; per net, a joint reads as a plain T. Where
  two nets cross, the later net's casing cuts the earlier one's core and
  they read as passing over each other. Members of one net in different
  colors (a splice net's conductors, a double crimp) overlap along their
  shared stub, where the later core covers the earlier one; the color
  changes where they split, with no black line between them. Code anchors
  are still placed wire by wire in connection order.
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

**Pin annotations** (`render/schematic/pin.rs`, `PinAnnotation`, class
`port-pin`) sit outside the box beside each port that has pins: the pins
(`ir::Pin::display_list`, `2,3` for a ganged port), after
`<designator>:` (`ir::ConnectorRef.name`) when the owning instance has more
than one connector (`Instance.connectors`, every connector of its
component, listed in the view or not, so a bare number always means the
only connector). A port on no connector (`bare_port_pin`) keeps its bare
pins. The enclosure's ports follow the same rule with the subject as owner;
an inline-connector instance has one connector and is never a schematic
box anyway. `Placement` fixes each text and its `PinOrientation`, so draw
and the canvas agree. A west or east text is level, above the stub,
running outward along it; only its height (10px) faces the next port, so
`MIN_PORT_PITCH` holds at any length. A north or south text is level, to
the right of the stub, while every text on that side ends clear of the
next port's stub (`PIN_INSET + width ≤ spacing − PIN_NEIGHBOUR_GAP`, with
`PIN_CHAR_WIDTH` per character; the spacing is the pitch, an enclosure
edge's closest pair, or none for a lone port); otherwise the whole side
turns to run along the stubs, right of a north stub and left of a south
one, so again only its height faces a neighbour and the two texts of a
short wire from a south port to a north port sit either side of it. A
turned text reads toward its port (`rotate(90)`, downward, on a north
side; `rotate(-90)`, upward, on a south one), so the pin number at its
end sits next to the port and the designator runs out along the stub
(an off-view reference on the same stub reads the other way, away from
the box, and starts past the text). The rule is the same on every
component: a single-connector side turns too when a bare text would not
fit level, such as a ganged `1,2,3` on the default grid or a two-digit
pin on a grid of 10. The minimum grid is therefore unchanged by long
designators. What a long text can still meet is what lies beyond the
stub's first grid step: the wire's own first bend, wires in the channel in
front of the box, and a `wire-code` (the router and the code placement do
not see annotations). The component title above a box is kept clear: a
north-side annotation whose area would meet `PlacedComponent::title_extent`
moves out along its stub (`PinAnnotation::clear_title`, a `lift` rounded
up to a whole unit) until its near end, the pin number of a turned text,
sits `PIN_TITLE_GAP` above the title; one clear of the title keeps its
place, and an enclosure's ports, which have no title over them, never
move. The move only lengthens the annotation's reach, which
`PlacedPort::pin_extent` reports, so off-view stubs still start past it
and the canvas still holds it. A wire that turns at the end of its
one-step stub leaves a moved text standing a few units past the bend,
beside no wire, and a horizontal run in the channel above can cross it. The
canvas (`Placement::viewbox`) keeps every annotation `PIN_CANVAS_PAD`
inside it, applied after the margin so a view whose annotations the
margin already holds keeps its canvas; `view_too_large` sees the same
canvas.

**Off-view references** (`render/schematic/off_view.rs`, always on, no
syntax). `Placement::compute` records on each placed port
(`PlacedPort.off_view`, an `Undrawn`) the ends its wires reach that the
view does not place, as `FarEnd`s, in wire order then endpoint order. Along
a wire, a placed port reaches the unplaced ends on either side of it up to
the nearest placed one (`[a, x, y, b]` with `a` and `b` placed gives both
`x, y`); a splice net is named once, beside the first port it places, as
the splice that port's conductor lands on and the net's unplaced ends; an
own end off the enclosure reads `out (leaves <Subject>)`, as in a pinout.
A port all of whose wires are drawn, or that has none, gets nothing. The
reference (`OffView`) is a stub in the colour of the port's first undrawn
wire, drawn with a wire's layers (classes `wire-casing off-view-stub`,
`wire off-view-stub`, `wire-tracer off-view-stub`, so a host's wire rules
stroke it and embed mode carries `stroke`/`data-color`), and at its end a
`.off-view-label` text, in a `<g class="off-view" data-component data-port>`
whose `<title>` lists every far end in full. Its stub runs out the way the
port's wire would leave (inward for an enclosure port), past the pin
annotations beside it and, on a box's north side, past the box's title;
the text continues on the stub's line, level on west and east (the arrow
at the stub's end: `→ vcm.can2_h` east, `vcm.can2_h ←` west), turned to
read outward on north (`rotate(-90)`) and south (`rotate(90)`). So the
port label stays inside the box, the pin annotation beside the stub near
the box, and the reference beyond both; neighbouring references are a
pitch apart across the stub and the text is 10px high, so they clear at
the minimum grid. Where a drawn wire also leaves the port, that wire turns
at its routing stub's tip, so the reference's stub runs a step past it.
The text is the most detailed of a ladder that fits: all far ends while
they fit `MAX_CHARS` (32), then fewer with `+N more`, then `N more`, and
last the arrow alone; a splice lists what it joins as far as room allows.
A rung fits when its space is clear of other boxes' keepouts, titles and
pin annotations, the text boxes, every port and routing stub, the
enclosure's boundary, and the texts of references placed before it;
with none fitting, the reference is *cramped*: it shows its arrow alone.
The canvas (`Placement::viewbox`) keeps each reference's text inside it
as it does a pin annotation.

The router keeps wires off a reference only when one would run through
it (`off_view::Settled::new`, which both `render` and `check`'s Layout
phase call, so they see one routing). The wires are first routed with no
reference; each reference a routed wire crosses (its stub and text, widened
by the halo and a wire's casing) is *reserved* — an obstacle to
`Router::build`, one grid-aligned rectangle from the port (from the stub's
tip when a drawn wire leaves the port too) to past the text, whole grid
steps either side of the stub's line — and the wires are routed again,
until no wire crosses an unreserved reference. A reservation that would
leave some connection without a path in the visibility graph
(`Router::connects`) yields: the reference is tried at its arrow alone,
and else stays unreserved. So reserving never takes a route away: a view
`check` passes without references it passes with them (its Layout
problems, `include_too_close` from `Router::blocked_stubs`, which sees only
the boxes, and `unroutable_wire`, are unchanged), and a view whose wires
miss its references routes exactly as before. A view that does not route
reserves nothing. Reservations join the nudging walls too, so a nudged
channel stays off them.

### Wire strokes (`render/wire_stroke.rs`)

The schematic and the harness draw a wire the same way, through one type:
a renderer names the classes of a wire's three layers (`WireLayers` —
`wire-casing`/`wire`/`wire-tracer` in a schematic,
`cable-wire-casing`/`cable-wire`/`cable-wire-tracer` in a harness), and
`WireLayers::stroke(&color)` gives a `WireStroke` whose `casing`, `core`
and `tracer` dress a bare shape (polyline or path) as that layer (the
renderer orders them: a wire drawn in pieces, a schematic net or a
harness strand, lays every casing before its cores, so no casing overdraws
a core at a join). The core carries its base
color as a `stroke` presentation attribute and the authored color as
`data-color` (`green/yellow`, synonyms normalised); the tracer carries the
tracer color the same way; the casing carries only its class. Widths, the
tracer's dash and the casing's black live in each renderer's `STYLE`
(schematic: casing 4, core and tracer 2; the stroke never affects routing
or the canvas), whose core rule sets no `stroke`: a presentation attribute
loses to any stylesheet rule. So in embed mode the color still shows with
no stylesheet rule for it, and a host stylesheet that sets `stroke` on the
core class (as one copied from the starter `wirebug.css` before wires were
colored does, `.wire { stroke: black }`) overrides it. Schematic layers also
carry `fill="none"` themselves, so such an older host stylesheet, which has
no rule for the casing or the tracer, draws no casing at all and a thin
solid tracer, rather than filling the area a bend encloses. A color name
outside IEC 60757 is painted verbatim: a CSS color name shows as itself,
anything else leaves the core unpainted, so the wire reads as its black
casing alone.

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
single connector is a jumper, never drawn — and each wire with an end on a
splice of the subject; an end is a `layout::HarnessEnd`, `Connector` or
`Splice`, keyed by an `ir::HarnessNode` of the same two kinds);
`HarnessLayout::compute` keeps those whose two ends' connectors or splices
are included and records them as `HarnessLayout.drawn` (`lint::Segment`,
wire and pair index), which is what `check`'s `undrawn_conductor` reads.
That scoping rule is written once, in the IR (`ir::HarnessScope`,
crate-internal, built from a harness view and the instance it is drawn
against, over `ir::HarnessNode`, what an end or an include lands on —
`HarnessNode::of_end` is the one lookup from a wire end to its connector,
and the layout's `HarnessEnd`s are built from it): `draws` (both ends'
nodes included, and distinct — `HarnessNode::between`, which also bounds
`HarnessConductor::all`), `draws_screen` and `draws_drain` (below), and
`half` (the half an inline is included through). The layout asks it,
and so does the pin-table export, which reads off it which housing half
of an inline connector a conductor is drawn into.

A **splice** include (`include sp`, an `ir::Include` with no connector
naming one of the subject's `Instance.splices`;
`HarnessNode::of_include`) draws as a `SpliceNode`: a solid dot at the authored point (`splice-dot`, haloed in
white so each conductor's end reads against it) with the name
(`splice-name`, bold), label (`splice-label`) and part (`splice-part`)
centred beneath, in a `<g class="splice">` drawn over the connectors — no
table and no outline, so it reads as a joint, not a connector. Every
conductor to it attaches at the dot's centre (`layout::Anchor::Splice`; a
pin's is `Anchor::Pin`), so they visibly converge there, and a pin wired
to a splice faces it as it faces a connector. A conductor to a splice
draws by the same rule as any other: a cable's through its cable box on
the spine, a loose wire as one bezier, or through a lane on the spine
where that bezier would meet a box (below). The splice's box (dot and text) is
`SpliceNode.bounds`, counted in the spine (`spine_x` takes every node's
and splice's bounds) and the viewBox, and reported as an `IncludeBox`, so
the overlap and canvas checks see it. One cable box per cable on one
spine still holds, so a splice placed between a cable's two ends draws
its strands doubling back; author the halves as two cables, or a cable
and a loose wire.

Kept conductors split by `Wire.cable`. A **declared cable** draws as a
**cable box** (`CableBox`/`render_cable_box`) on the spine: a titled table
(label + `type · length · ×count`, the length in metres as
`ir::Length` displays it, `0.15 m`), one coloured strand per row. A
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
box's vertical centre is its strands' centroid (`layout::midpoint_y`).

**Loose wires** (`LooseWire`/`render_loose`, a conductor in no cable)
draw as one bezier pin to pin, annotated at its midpoint — unless that
curve, or its annotation, would **meet a cable box**
(`LooseWire::meets`: the curve sampled against the box grown by half the
wire's stroke, the annotation's text box against the box), where it would
read as that cable's conductor or be hidden by it. Then a wire whose two
ends lie either side of the spine (`Crossing`) gets a **lane**
(`layout::Lane`) there instead: a row of its own in the spine's
occupancy, like a one-conductor box without the box. The laned wires
between one pair of nodes (connector or splice, `AnchorNode`) form one
`LooseSlot`, a lane each, ordered by midpoint y and centred on their
centroid as a box's rows are; boxes and slots are stacked along the spine
together (`Occupant::stack`: sorted by top edge, ties in source order by
each one's first conductor, swept top-down, gap `CABLE_GAP`). A slot can
push a box onto another wire's curve, so `HarnessLayout::occupy_spine`
repeats — boxes back home, the laned wires' slots stacked with them, the
remaining curves tested against the boxes as they now stand — until no
further wire needs a lane; wires only ever go from direct to laned, so it
ends, and a view with no cable box draws every loose wire exactly as a
bezier. Each lane's straight run is centred on the spine and spans the
widest box or its own annotation, whichever is wider
(`LooseWire::take_lane`), so its leads end beyond every box's edge and,
staying within their endpoints' x-span, never enter a box; the
annotation sits over the run's middle (`LooseWire::annotation_at`). The
run is kept inside the gap between the wire's ends, short of
`LANE_MIN_LEAD` each side, where the widest box allows: an annotation
wider than that is cut short with an ellipsis (`draw::shorten`,
`Lane::fit`), the full text in a `<title>`. Loose wires draw over the
boxes, so one that still meets a box — it stays on one side of the
spine, which only happens when one of its ends lies inside the box
column — crosses it visibly rather than hiding behind it. The lanes are
the only avoidance: leads still cross other leads, and stacking only
pushes down, so a slot can push the boxes below it (and their leads)
further down. Lanes add their run's ends to the viewBox, so the canvas
`check` sizes (`viewbox`) grows with them. A loose wire is drawn as one
path per layer (`Cubic::chain_d`, class `cable-wire`, in a
`<g class="cable">`).

Every wire segment is one horizontally-flexed cubic bezier (`bezier.rs::flex`,
`FLEX = 0.4`): a cabled conductor is lead-in → straight box run → lead-out,
and so is a loose wire in a lane (its run a flat cubic); any other loose
wire is one curve. Control points share each endpoint's y and stay
within the endpoints' x-span, so the curve never overshoots its bounding box
(no viewbox padding needed). Each strand is stroked with `wire.color` (the SVG
`stroke`) over a wider black casing path (WireViz's trick, so light
colors read on any background; see Wire strokes) and annotated
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
and wire routing is the bezier flex above — no object avoidance beyond a
loose wire's lane, unlike the schematic's orthogonal router.
**Shields** draw with their cable (`layout.rs`, `DrawnShield`): a cable
box is drawn only for its conductors, as before, and then carries the
screens of its cable — the overall one, and a pair's own when that pair is
drawn. An overall screen marks the box with a dashed inner outline
(`cable-shield`) and `shielded` after the conductor count in the subtitle;
a screened pair gets a dashed rounded outline round its rows
(`pair-shield`, `CableBox::pair_rows`). Each shield end on a connector the
view includes is a **drain**: the pin joins its table's auto-scope (facing
the spine, since it carries no conductor of its own) and the shield gets a
row of its own in the box (`DrainRow`; a pair's right below the pair, the
overall one last, every row placed by its `row` index), drawn as a thin
dashed `drain-wire` across the box and a flexed `drain-wire` lead from the
nearer box edge to each pin, with an italic `drain-label` (the shield's
label, else `Shield`/`Pair shield`), all in a `<g class="drain">`. An end
on a connector the view leaves out is not drawn; a shield on no pin
has no row, only its outline. A shield alone never puts a cable
box in a view. Reusing the orthogonal router is a noted future
refinement.

### Pinout views (`render/pinout/`)

An include names one of the subject's own connectors (never an inline
connector's half: an inline is a child instance of its own, not one of the
subject's `connectors`); it becomes one pin table centred at its grid
point. Like the other views it documents a type and is drawn against
`Design::subject_of` (the first instance). The table is a crimping sheet
read off the connector's `export::PinTable` (see Build tables): the
design's `PinTables` are built once per run — `render_views` and
`lint::layout_problems` each hold a `OnceCell` the first pinout view
fills — and each include finds its table with `PinTables::of` (instance
and designator; an ordinary connector, so `half` and `HalfFit` never
arise). The header (`Header`) has a bold title, the component and the
designator, `Inverter · control`: the component is that instance's label
only when the type is placed exactly once (`Design::placements_of`), else
its type name, since a label would name one placement of a sheet that
serves them all. Under the title come the header's lines: the housing's
description and `part`, each left out when missing (and the line with
them), then, for a type placed more than once, `Wires shown for
<instance path>, one of N placements` — the wire columns are a
placement's, the first's. `Header::height` grows by a line height for
each line past the first, and every size below the header follows from
it.

The rows (`rows::Row`, `Row::all`) are one per cavity a port is bound to,
in the `PinTable`'s order (`ir::Pin`'s numeric `Ord`, not declaration
order; a ganged port has a row per pin), then one per `PinTable.unpinned`
port with an empty pin cell, so a conductor into a port with no pin is
not lost from the sheet. A layout cavity no port uses has no row (the
face shows it). Columns: pin, port label (with a right-aligned note,
`rows::Note`: `ganged 46+48` on each row of a ganged port, whose
conductors the model lists in each of its cavities, or `no pin`), then
one sub-row per `CavityEntry` (`rows::Entry`, `SUB_ROW_HEIGHT` each past
the first, a dotted `wire-sep` between): a conductor's colour swatch and
IEC code, gauge (`0.5mm²`) and far end, or a drain (`drain`, no colour
or gauge; the far column names the screen — the cable and the shield's
label, else `shield` / `pair N shield` — and its other end for a screen
grounded at both). A bound pin with no entry shows a dash
(`pin-unwired`). The far end (`rows::FarText`) is a list of texts, most
detailed first: a port's full path with `(connector:pin)`, then without;
a splice's path with what its net joins beyond it, named from the splice's
component and collapsed to `+N more`, then the bare splice. A
`FarEnd::Boundary` on one of its component's connectors reads like any
port (a real termination on a pin); one on no connector is where the
conductor leaves its component for the parent's wire to carry on, so it
is marked, `→ Vehicle.loom.out (leaves Loom)`, falling back to
`→ out (leaves Loom)` — never shown as an ordinary destination. Where
it continues is not followed (the export's wire list has it). The swatch
(`swatch`) is painted as the harness paints a strand — `WireColor::css`
for the base, `WireColor::tracer` as a stripe — with the authored colour
as `data-color` in every mode; it is a `<path>`, since `.pinout rect`
would paint a rectangle's fill white.

Each column (`Columns`) is as wide as its widest cell, measured with the
same estimator as cavity labels (`fit::Font`, per-class widths, bold
counted wider), with minimum widths so an unwired table still shows its
columns; the far column is capped at `FAR_MAX_WIDTH`, then narrowed to
the widest far end as drawn there. A far end takes the most detailed text
that fits its column and is cut with an ellipsis only when the least
detailed one does not; a shortened one keeps the whole text as a
`<title>`. The table's width is its columns' (or its header's, if
wider), independent of the face. When the connector type has a `layout`
(`grid` or `face`), its cavity face sits in a panel of its own
(`face-panel`) on top of the table, as wide as the face with its padding
or its caption band (`FACE_CAPTION`, "Viewed from harness side": the
language authors every layout from the harness side and has no notion of
another), and the two are centred on the include's point; the include's
box is their union (`Table::bounds`). A face cavity no port is bound to
keeps the `cavity` class and adds `cavity-unused` (its pin
`cavity-pin-unused`): dashed, grey and lightly filled. A port bound to a
cavity counts as used whether or not a conductor reaches it — the view
documents the type, the conductors belong to a placement. A cavity label
is fitted into its cavity (`fit::FittedLabel`): text has no clipping, so
its width is estimated per character class from DejaVu Sans, the widest
common `sans-serif` face, and the label shrinks in half-pixel steps from
8 px down to a 6 px floor (an inline `font-size` style, since the
stylesheet's `font` shorthand would beat an attribute), then is cut with
an ellipsis; a cut cavity carries the whole label as a `<title>`, and the
row shows it whole. Every size — header, face panel, caption band, rows
and sub-rows, columns — comes from `Table::new`, which both `render` and
`layout` (the overlap and `view_too_large` checks) use, so the drawing
and `check` agree.

## Build tables (`export/`)

The tables a harness is cut, crimped and ordered from are typed models
built from `ir::Design` alone: no layout, so they list what the design
holds whether or not a drawing shows it (the pin tables read the harness
views only to tell an inline connector's halves apart). They borrow the design
(`'d`) rather than copy it, and carry no serialisation: writing them out
is `export::records` (below), and the IR is not a wire format.

- **Wire list (`export::WireList`, `From<&Design>`)** — the cut list.
  One `export::Row` per physical conductor: each `Wire::segments()` run
  of each wire, so a rail of four ends is three rows and two wires into
  one cavity (a double crimp) are two. A cable screen is a row too, of
  the other `RowKind`: `RowKind::Conductor(Conductor)` (the `Wire`, its
  `WireSegment` and two resolved ends) or `RowKind::Shield(ShieldRun)`
  (what it screens, the `ir::Shield`, and its `Drain` — `NoPin`, `Single`
  or `Both`, the shape of `ShieldTermination`) — so a drain lands on a pin
  like a conductor does and a pin table built from the list sees it.
  Every row has its `owner` (the instance whose body declares it), its
  `Cable` (designator and `CableMeta`) when it runs in one, and the
  length it stands for, an `export::RowLength`: `Own(Length)` for a
  two-ended conductor (`Instance::effective_length`) or a screen (its
  cable's), `Unset`, or `Rail { total, first }` on each segment of a
  wire with more than two ends — a rail's `length` is the whole rail's,
  not each leg's (a cable conductor is two-ended, so a cable's length
  never meets a rail). `RowLength::copper()` gives what a row adds to a
  total (a rail's on its first segment only), and `WireList::copper()`
  sums the conductors so each rail counts once, as an `export::Copper`
  (the summed `Length`, which adds exactly, and how many conductors have
  none); screens are not counted.
  An `export::End` is typed: `Port` (a port of an instance placed in the
  owner, inline connectors included), `Boundary` (the owner's own port,
  where the conductor leaves it), `Splice` (one of the owner's), or
  `Unresolved` (only in a design with errors, so no row is ever dropped).
  A port end (`PortEnd`: instance and `ir::Port`) says where it is
  crimped through `PortEnd::contact()`: `Contact::Connector { designator,
  pins }` (several pins for a ganged port; an inline connector's
  designator is its name, and the IR does not say which half a conductor
  is crimped into) or `Contact::Connectorless`, which keeps its row. Ends
  are named relative to the owner in the segment (`WireEnd`, as
  authored), and an `End` displays as the full path of what it lands on
  (`Vehicle.loom.lamp.pwr`). The list covers every instance in IR order,
  so a type placed twice lists its rows twice with distinct owner paths;
  within an instance its wires come in declaration order, then its
  cables' screens in cable order (overall before a pair's). Nothing in it
  iterates a hash map, so the order is the source's.
- **Pin tables (`export::PinTables`, `From<&Design>`)** — the crimp
  sheet, built from the wire list: every conductor end and drain end the
  list lands on a connector pin is entered in that pin's cavity. One
  `export::PinTable` per connector of each instance (the root's and every
  placement's, so a type placed twice has its tables twice; a
  connectorless port has none), in IR then connector order; an inline
  connector is a mated pair, so it has a table for each of its two halves
  (`PinTable.half`, male first), whether it declares that half or not
  (`PinTable::is_declared`: an undeclared half has no layout, so its
  cavities are the inline's pins). A table's
  `export::Cavity` rows are sorted by `ir::Pin`'s `Ord` (numeric; pins are
  whole numbers in the DSL), one per pin a port is bound to plus one per
  cavity of the connector's `layout` (an inline half's own layout) that no
  port uses (`Cavity.port: None`), so an unused cavity is listed empty. A
  port of the connector with no pin (the DSL lets a connector port leave
  its pin out while the pin-out is unknown) is not a cavity: it is an
  `export::UnpinnedPort` (port and entries) in `PinTable.unpinned`, in
  source order, so a conductor into it is listed rather than dropped and
  code that sorts or counts cavities never meets a missing pin.
  `PinTable::housing()` is what the table's housing is bought as, an
  `export::Housing` (connector type, description, `part()`): an inline
  half's connector type, whose metadata is not on the synthetic
  connector (for an undeclared half, a housing of no type described by
  the inline's label), else the connector's own. A
  ganged port (`pins [2, 3]`) is entered in each of its cavities, marked
  by `Cavity::is_ganged` and `Cavity::ganged_with` (the other pins), since
  the model does not say which cavity takes which conductor. A cavity's
  `entries` are in wire-list order (two for a double crimp), each an
  `export::CavityEntry`: `Conductor(CavityConductor)` (owner, `Cable`, the
  wire list's `Conductor` and its `FarEnd`) or `Drain(CavityDrain)` (the
  screen's `ShieldRun` and, grounded at both ends, the drain's other end),
  never a conductor with an invented colour. A `FarEnd` is the end across
  the conductor: `Port` (its `contact()` says pin or connectorless),
  `Boundary`, `Unresolved`, or `Splice(FarSplice)` — the splice and, as
  `SplicedEnd`s (end and wire), every other end its `Instance::splice_net`
  joins, linked splices followed and the conductor's own end left out.
  Each entry carries an `export::HalfFit`. The IR does not record which
  half of an inline connector a conductor is crimped into; the harness
  views say it, by `ir::HarnessScope`, the scoping rule the harness
  renderer draws by: an end on an inline goes to the half the harness
  views of its owner's component draw it into (`Assigned`, and absent
  from the other half's table) when they draw it into exactly one; when
  none draws it, or views draw it into each half, it is listed under both
  halves as `Undetermined`, never guessed or dropped. The rule is the
  same whatever the inline declares: a view can include only a declared
  half, so an undeclared half is never assigned anything, and a conductor
  no view draws into the declared half is undetermined, not assumed to be
  in it. A drain is placed the same way (`HarnessScope::draws_drain`).
  Elsewhere an entry is `Assigned`. `PinTables::of` (crate-internal) finds
  one ordinary connector's table by instance and designator; the pinout
  renderer reads its rows from it.
- **Bill of materials (`export::Bom`, `From<&Design>`)** — what is
  bought: one `export::Line` per thing to buy, a closed enum whose
  variant is its kind (`Line::kind()`, an `export::LineKind`, whose order
  is the BOM's: housings, cables, loose wire, components, splices) and
  holds the line's placements, so `Line::quantity()` is their count — per
  placement, since every source it reads is (a type placed six times
  counts six). Lines within a kind are grouped by an `IndexMap` key (a
  private `Groups` collector), so they come in order of first use and no
  hash order reaches the output. `HousingLine` reads `PinTables`, one
  housing per table (`PinTable::housing()`), so an inline connector is
  always two housings, as it is two tables: a half it does not declare is
  an `export::Housing` of no type described by the inline's label.
  Housings are keyed by part, else
  connector type, else description (a connector block naming no type),
  else nothing (one line). `CableLine` reads every
  instance's `cables`, keyed by `CableMeta.type`; its `length()` sums the
  cables' own `length`, not their conductors'. `WireLine` reads the
  `WireList`'s conductor rows in no cable, keyed by colour, gauge (its
  bits), `type` and `part`; its `length()` collects the rows'
  `RowLength`s into an `export::Copper` (`FromIterator<RowLength>`, which
  `WireList::copper` uses too), so a rail counts once; its `boundary()`
  counts the rows with an `End::Boundary` end on a `Contact::Connectorless`
  port — a wire-through continuation of the parent's conductor, kept in
  the quantity but flagged (a boundary port on the owner's own connector
  is a real termination). `ComponentLine`
  reads the instances, keyed by `PartMeta.number`, else by type; the
  root, inline instances and, without a part number, assemblies
  (`Instance::is_assembly`: children and no port or connector of its own)
  are left out. `SpliceLine` reads every instance's `splices`, keyed by
  part, else by label (else name). `Line::is_specified()` says whether a
  line names what to buy (a cable's `type`, which a cable body has in
  place of a part); `Line::length()` is the `Copper` of a cable or wire
  line, whose `unmeasured` says what its total leaves out. Contacts,
  terminals and seals are not in the model, so not in the BOM; a screen
  is part of its cable.
- **Wire format (`export/records/`)** — what `wirebug export` prints and
  `render --embed` writes. Each table becomes record types private to the
  module, built from the models (`WireSheet` from a `WireList`,
  `ConnectorSheet` from `PinTables`, `BomSheet` from a `Bom` — one flat
  `LineRecord` per line, every kind under the same fields, so its JSON
  and CSV are the same table; `EndRecord` for an end in either,
  `From<&End>` / `From<&FarEnd>`), owned strings and numbers with serde
  derives: the JSON nests them, and the CSV is their flattening
  (`Sheet::csv_rows`, one row per wire-list row or per cavity entry, under
  `Sheet::HEADER`), so the two formats carry the same data by
  construction. The JSON document wraps a sheet's fields (`rows`,
  `connectors` or `lines`) in `table`, `format_version` (`export::FORMAT_VERSION`),
  `project` and `generator` (`crate::GENERATOR`, the embed manifest's).
  CSV goes through the `csv` crate (RFC 4180 quoting, `\n` endings, and a
  row of the wrong length is refused). The rail rule is explicit:
  `length_m` is `RowLength::copper()` (a rail's length on its first
  segment only, nothing on a screen), and `rail_length_m` /
  `rail_segment` carry the whole rail on each of its segments, so summing
  `length_m` is `WireList::copper()`. Twisted pairs are numbered from 1
  (the IR's `twisted_group` + 1). The columns are a public interface and
  are listed in README's "Exporting build tables"; a change to one's
  name, place or meaning raises `FORMAT_VERSION` once the table has been
  released; a new table does not (adding the BOM left it at 1), and nor
  did the BOM's `boundary` column, added before the table was released. A new table is a new `Table` variant, a
  `Sheet` impl and an `EmbedExports` field.

## DSL validation (`check`)

Problems are miette `Diagnostic`s (`dsl::diagnostics::Problem`), collected
so one run reports many. Errors fail the run; warnings fail only under
`--strict`. The checks, by phase:

- **Load** — file not found for a `use` or a `fragment`
  (`use_not_found`, on the path); no `wirebug.toml`; IO.
- **Parse/lex** — syntax and lexical errors (with expected-token sets),
  a unit on a number where no length goes (a gauge, a coordinate, a
  `grid:`, a pin, `rows`/`cols`, a `connector_type` property: the bare
  `number` and `whole` productions refuse it, naming the number to write;
  only a property value parses one, `quantity`), and a length unit after
  a space (`length: 2 m;`, which says to write the unit against the
  number in place of "expected ';'"),
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
  since the wire body is a block of its own; a broken `shield` line in a
  pair skips the pair, reporting the shield's own error, never "a twisted
  pair needs exactly two conductors", since the group's braces fail to
  close before the count is taken). Two guards
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
  or unused imports and unused fragments (but for a `fragment` naming its
  own file), no root, a view of nothing, an
  empty cable, a splice that joins nothing, a port without `pub` — a `;` too early parts `pub ; port a "A";` into a skipped
  `pub` and a private port) are dropped (`Project::drop_parse_cascades`,
  in `dsl::check_loaded`, so for the CLI and the LSP alike), and come
  back once the syntax is fixed. The problems that report something present and wrong (a bad
  gauge, a duplicate, an unknown color) are still reported: skipping a
  statement cannot cause them. Such a project yields no design
  (`check_loaded` drops it), so it never renders.
- **Resolve** — undefined type, unresolved import, duplicate
  type/instance/port, unknown instance/port in a wire endpoint,
  private-port access (a non-`pub` port referenced from outside), unknown
  view include, a view without `for` in a file with no or several
  top-level components (`view_subject`; the help shows `for`), a `for`
  naming no component in scope (the instances' `undefined_type`, on the
  name) or a connector type (`view_subject_not_component`), duplicate
  connector designator (`duplicate_connector_name`), duplicate cable designator
  (`duplicate_cable_name`), a splice named like a port, instance, inline
  connector, connector, cable or other splice of its merged component
  (`duplicate_splice_name`, one per splice, on the splice; the other
  declaration a second label in the same file, else a `#[related]`
  `OtherDeclaration` note; `Resolver::check_splice_names`), a shield end
  naming a splice (`shield_on_splice`). An unknown bare conductor endpoint
  is the usual `unknown_port`, its help saying a bare name is an own port
  or a splice. `extend` fragments add: a fragment nested inside a
  component (`nested_extend`); an `extend` whose merge group has no root
  `component` (`orphan_fragment`), its `diagnostics::OrphanFragmentCause`
  telling `Undefined` (no `component` of the name anywhere) from a
  component that exists but is not in scope where the file is pulled in:
  `PulledInWithoutIt` when one file defines it and one file's line (a
  `fragment`, or a `use` of the name) merges the `extend`'s file, the help
  naming the `use` to add there with its path (`Project::import_path`),
  else `NotInScope`, naming the defining file when there is one; an
  instance/port name declared in two fragments of
  one merged component (the ordinary `duplicate_instance`/`duplicate_port`,
  reported once across the group). A cable's wire endpoints resolve exactly
  like a loose wire's. A view kind no renderer implements is
  `unknown_view_kind` (so `check` catches it; render's own guard is a
  backstop for hand-built designs). View `ports { }` placements get the
  same treatment as wire endpoints: unknown side (`unknown_port_side`),
  unknown/private port, and a duplicate-port-in-one-include guard
  (`duplicate_view_port`). Includes are checked per view kind: a `harness`
  include must name an existing connector (`unknown_connector`) and carry no
  `ports { }`, or name one of the subject's splices alone (`include sp`, no
  `.connector`, no `ports { }`; `check_splice_include`); a `schematic`
  include must not name a connector, and no schematic or pinout include may
  name a splice — violations are `wrong_include_form`. A splice included
  twice in one view is a `duplicate_view_include`. A view's `enclosure { }` ports resolve against the
  *subject*: each anchor must name exactly one side in the slot for its axis
  (`enclosure_anchor`) and an existing `pub` subject port (`unknown_port` /
  `private_port`), with the same duplicate guard (`duplicate_view_port`).
  A `text` box outside a schematic view is `unsupported_view_text`; two
  boxes of one view sharing a name are `duplicate_view_text` (only the
  deprecated named form has a name to clash).
  As warnings: a `fragment` that adds nothing (`unused_fragment`, on the
  path; a stale line, in the spirit of `unused_import`). A line is useful
  when it merges one of its target's top-level `extend`s, or when the
  target contributes something: a view, a merge its own scope makes (a
  `use` collision or one of its `fragment` lines), or a `fragment` line
  naming a file that contributes. A file's views count only when none of
  its `extend`s is orphaned: views of a component the file never joined
  document nothing that is there, so a line naming a loom file whose
  `extend` is not in scope is reported even though the file has a harness
  view. Files already loaded on the way — the
  entry file, the file the line is written in, the files followed to get
  there — contribute nothing again (`FragmentGraph::contributes`, a search
  with a visited set; `build_scopes` returns the lines and
  `Resolver::report_fragment_lines` judges them once the merge groups are
  final, after `report_orphan_fragments`), so two files that
  only name each other are both reported, and an aggregating file of
  `fragment` lines that `use`s their component is not. A line naming its
  own file is always reported. The variant carries why
  (`diagnostics::UnusedFragmentReason`): `Empty` (no view, `extend` or
  `fragment` line), `ItsFragmentsAddNothing` (only `fragment` lines, none
  adding anything), `NotInScope` (naming the extended components missing
  from the naming file's scope, the help telling to `use` them there or
  move the line), `Imported` (the scope entry of the extended name is that
  same `extend`, brought in by a `use` of the file, so there is no
  component to merge it into), `TheEntryFile` or `ItsOwnFile`. All but
  `ItsOwnFile` report an absence, dropped while a file has syntax errors
  (`Problem::reports_an_absence`); a line naming its own file is wrong
  whatever a skipped statement held. A second
  `fragment` in one file naming a file an earlier one did, compared by the
  loaded file so `./x.wb` repeats `x.wb` (`duplicate_fragment`, on both
  paths; a `use` of the same file is no repeat, since a file loads once
  whatever names it); a text box in the deprecated named form
  (`deprecated_text_syntax`, labelled on the statement, its help the
  statement rewritten label first by `ast::TextBox::current_form`); and a
  view title already used by another view anywhere in the
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
  `cable_wire_arity`; a loose wire with a splice among more than two
  endpoints, `splice_wire_arity`, since a conductor to a splice runs end
  to end); property checks on a cable body and on any wire's
  body and on a component's part data (`validate_properties`, keyed by
  `diagnostics::PropertyOwner`, whose `keys()` say what each body takes: a
  cable `type` and `length`, a wire `length`, `type` and `part`, a
  component `part`, `manufacturer`, `description` and `rating`, a splice
  `part` and `description`, a closed set the bill of materials reads) — an unknown key, a key set twice, a
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
  conductor's gauge, `invalid_length` on a cable's or a wire's `length` —
  under a micrometre included, the finest an `ir::Length` holds — both
  labelled on the number, a length's unit included); a `length` whose
  unit is none of `m`, `cm`, `mm` (`unknown_length_unit`, labelled on the
  unit, the help listing the units; a unit on a key that takes a string
  is the usual `cable_property_type`); an endpoint listed twice in one wire or cable
  conductor (`duplicate_endpoint`, labelled on both occurrences — compared
  as resolved `ir::WireEnd`s, so own port `gnd` and `ecu.gnd` never alias;
  two *different* wires on one port are a deliberate double crimp and are
  not compared); a shield listing an end twice is the same
`duplicate_endpoint`, its message naming the shield, and a shield end
sharing a pin with a wire is not compared either (a drain crimped with a
ground wire). A second `shield` in one cable body or one `twisted { }`
group (`duplicate_shield`, on both statements, the message naming the cable
or the pair), and a shield listing more than two ends (`shield_arity`, on
the statement: a screen, like a conductor, runs end to end), are errors;
the shield ends resolve in resolve like wire endpoints. As warnings: a
cable with no conductors (`empty_cable`; a shield is no conductor), a
splice fewer than two conductors of its merged component end on
(`splice_joins_nothing`, on its name; two is a butt splice;
`validate_splice_conductors`),
  unused import (an import counts as used when the file instantiates it,
  merges with it, a `fragment` of the file merges an `extend` into it, or
  the file names it in a view's `for`), bare-port pin, and
  non-IEC 60757 wire color
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
  would draw it, as `lint::EndTarget`s — a splice as itself
  (`IncludeTarget::Splice`), the help then saying a view must include what
  both ends land on, connector or splice; an inline connector only through
  the halves it declares, and one that declares none is told to declare a
  half first, since an include of an undeclared half is an error). Each harness view's `ViewLayout.drawn` is the
  `HarnessLayout.drawn` it renders; `lint::HarnessCoverage` unions them per
  subject type, and every `HarnessRenderer::conductors` entry (the same
  `HarnessConductor::all` candidates) outside the union is reported. So a
  multi-endpoint rail is judged per chain segment, a conductor to a splice
  is drawn when a view includes both its ends, an inline connector is
  covered through either half (an include of either normalizes to the
  designator), a double crimp is two conductors and never a problem, and
  a component with no harness view is not built from drawings yet, so is
  never reported. Shields are not conductors and are not judged: a drain no
  view draws is not reported. A harness view with a rejected `grid:` still counts its
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
  the `--embed` manifest (`EmbedManifest`); and what stops an export (or
  `--embed`'s tables): writing a table's CSV (`ExportCsv`) or JSON
  (`ExportJson`), each naming the `export::Table`, and `Write` for its
  file.

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
  lists only those (a host never sees a view whose file is missing). The
  build tables are written before the manifest, so it never names a
  missing table either. The failures are on stderr only.
- **`--pdf`** — `PdfExporter::to_pdf` takes `render_views`' results and
  makes one page per view that rendered and converted, numbered
  `n / total` among themselves; `PdfExport::failures` names the rest, in
  view order. `PdfExport::document()` is the only way to the bytes and
  returns `None` when every view of a design with views failed, so no
  PDF is written then. A failed view gets no `SmallText` warning; one
  whose text prints too small is a warning, not a failure, and keeps its
  page.

In every mode `render` prints `error: view "<title>": <cause>` per failure,
reports `rendered <n> of <total> view(s)`, and exits non-zero. A file left
in `--out` by an earlier run of a now-failing view is not removed.

`render` runs `check_project` first and refuses to render a project that
has errors (or, under `--strict`, warnings).

## Source tree

```
src/
├── main.rs          # clap CLI: `check`, `render`, `export`, `serve`, `lsp`, `manifest version` (all over the .wb DSL)
├── lib.rs           # re-exports; check_project + render::render_views
├── pipeline.rs      # check_loaded: dsl::check_loaded, then render::lint
│                    #   (the layout checks) — shared by check_project and lsp
│
│  # ── build tables derived from ir::Design (no rendering) ──
├── export/
│   ├── mod.rs       # re-exports the table models and Export
│   ├── wires.rs     # WireList: one Row per conductor segment and per screen
│   ├── pins.rs      # PinTables: one PinTable per connector (per inline half),
│   │                #   one Cavity per pin + unpinned ports, built from the WireList
│   ├── bom.rs       # Bom: one Line per thing to buy, per placement, grouped by part
│   └── records/     # the wire format: Export (Table × Format → text), Exported
│       ├── mod.rs       # Table, Format, Export, the JSON Document, CSV writing
│       ├── ends.rs      # EndRecord: an end as both tables write it
│       ├── wires.rs     # WireSheet: the wire list's records and columns
│       ├── connectors.rs# ConnectorSheet: the pin tables' records and columns
│       └── bom.rs       # BomSheet: the bill of materials' records and columns
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
│   ├── project/mod.rs   # walk-up discovery + transitive `use`/`fragment` loading → Project
│   ├── resolve/         # DefId registry + reference checks, one pass per file
│   │   ├── mod.rs       # data model (Resolved, *Facts) + the resolve() driver
│   │   ├── register.rs  # pass 1: AST walk → DefInfo facts, local duplicates
│   │   ├── scopes.rs    # per-file type scopes (own defs + `use` imports),
│   │   │                #   `fragment` merges, unused/duplicate fragments
│   │   ├── merge.rs     # MergeGroups: `extend` canonicals, orphans, dup guards
│   │   ├── refs.rs      # flat-namespace lookups + instance/connector/inline/
│   │   │                #   endpoint reference binding
│   │   └── views.rs     # view subject binding + include/enclosure checks
│   ├── elaborate/mod.rs # AST/registry → ir::Design; containment-cycle guard
│   ├── ir/
│   │   ├── mod.rs       # id newtypes + elaborated Design/Instance/Port/Wire/View
│   │   └── harness.rs   # HarnessScope: what a harness view draws, through which
│   │                    #   inline half (shared by render/harness and export)
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
│   ├── pdf.rs       # SVG → one multi-page PDF for --pdf: Paper, the page fit,
│   │                #   SmallText (views too small to read on the paper)
│   ├── png.rs       # SVG → PNG rasterisation for --png (resvg/usvg/tiny_skia)
│   ├── stamp.rs     # project-identity stamp text/element (corner of every SVG)
│   ├── wire_stroke.rs # WireLayers/WireStroke: a wire's casing, colored core
│   │                #   and tracer, shared by schematic and harness
│   ├── schematic/   # rectangle-based SVG renderer (kind: schematic)
│   │   ├── mod.rs       # SchematicRenderer; render orchestration
│   │   ├── layout.rs    # Placement: derive sides + boxes/ports in world coords
│   │   ├── draw.rs      # SVG emission (named `draw`, not `svg`, to
│   │   │                #   avoid clashing with the `svg` crate)
│   │   ├── pin.rs       # PinAnnotation: a port's pin text (designator-
│   │   │                #   qualified on multi-connector boxes), its
│   │   │                #   orientation, anchor and extent
│   │   ├── off_view.rs  # OffView: where a port's undrawn wires go (stub +
│   │   │                #   text), and Settled: routing reserved round them
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
│   │   ├── layout.rs    # conductors (HarnessConductor), pin-table nodes, splices,
│   │   │                #   spine + facing, cable boxes (centroid placement
│   │   │                #   + de-overlap), loose wires and their spine lanes
│   │   ├── bezier.rs    # horizontally-flexed cubic bezier math (FLEX)
│   │   └── draw.rs      # SVG emission: pin tables, cable boxes, bezier wires
│   └── pinout/      # connector-face + pin-table renderer (kind: pinout)
│       ├── mod.rs       # PinoutRenderer; cavity faces from connector layouts,
│       │                #   the table's columns, swatches and layout
│       ├── rows.rs      # Row/Entry/FarText: the table's rows from a PinTable
│       └── fit.rs       # FittedLabel, Font: text shrunk or cut to its width
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
- A number token carries the unit written against it: the name-shaped run
  touching its digits (`Token::Number { digits, unit }`, `150mm`), one
  token spanning both. So a number directly followed by a name is always
  one token, whatever the name spells (`1at` is no pin); after a space the
  name is a token of its own. The lexer judges no unit: the parser says
  where one may go, validate which ones exist.
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
  plus `from`, `for` (a view's subject), `fragment` (a top-level
  statement, where no name can start one), `shield` (a statement of a cable
  body or a pair) and `splice` (a component-body statement when a name
  follows it; `splice: Sensor;` reads on as an instance) — mean something only in their
  own production; everywhere a
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
- Files are loaded once each (by canonical path), so an import cycle or a
  diamond import (`use` and `fragment` alike) is harmless and never
  double-reports. Directory layout never affects logical hierarchy — only
  `use` and `fragment` paths and DSL nesting do.
- Wire endpoints are at most two-part (`inst.port`, or a bare own port or
  splice); the deep dotted form is an IR *path*, not surface syntax.

## Connector routing

Connector routing follows the orthogonal-routing paper (§4–6).
Nudging (§6) is implemented for the case our router produces — wires
sharing *collinear* channels (straight bundles): §6.1 orders a channel
by where each route enters it, and §6.2 spreads segments with a VPSC
solver that pins port ends and adds wall constraints keeping interior
segments outside the clearance-inflated boxes. Walls outrank pins, and
pins outrank free segments, as strength tiers (`vpsc::Strength`), not
large weights: a block sits exactly where its strongest members put it,
so a segment pushed against a wall lands on the wall's grid line rather
than a weight's rounding error off it. Not implemented: the
paper's general branching-tree ordering (pseudo-direction + split
points) and alley-midpoint recentring. Revisit if a view needs them.

Routing is deterministic: the same view gives the same bytes in every
run and every process, and `check`'s Layout lints, which route with the
same `Router`, see the same routes. Nothing on the way iterates a hash
map. Two choices settle the ties a symmetric layout produces:

- **A\* ties.** Two routes of equal cost (a wire going round a box on
  either side) are settled by the order A\* is handed a node's
  successors, so the visibility graph lists every node's neighbours in
  `Dir` order (north, south, east, west). In practice a wire that must go
  round a box goes east rather than west, north rather than south. The
  graph keeps its cells in a dense `Vec` in coordinate order, not a map.
- **Nudging order.** `nudge::segments::shared_edges` is an `IndexMap`, so
  channels, and the separation constraints built from them, come in the
  order the routes (in connection order) first reach them; the solver
  breaks equal violations by constraint order.
- **Off-view reservations** are tried in placement order (boxes in include
  order, ports in side order, then the enclosure), each round of reserving
  over the references the last routing crossed.

`tests/fixtures/tied_routes_project` holds the mirror-image ties; a
test renders it repeatedly in one process and a CLI test in several.

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
- [`pdf-writer`] — assembles the multi-page document around `svg2pdf`'s
  chunks. Pinned to the version `svg2pdf`'s API exposes (its `to_chunk`
  returns this crate's types) — bump the two together.
- [`serde_json`] — serialises the `--embed` `manifest.json` sidecar and
  the exported tables' JSON.
- [`csv`] — RFC 4180 writing of the exported tables (`export/records/`):
  quoting of labels with commas, quotes and line breaks, and a check that
  every row has a field per column.
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
[`csv`]: https://docs.rs/csv
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
