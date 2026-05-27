# wirebug — working notes

User-facing intro is in `README.md`. This file is for context that helps
future work on the codebase. Keep it short; don't restate the README.

## One pipeline

The only input is the **`.wb` DSL** (spec: `.github/skills/wirebug-dsl/`).
Two CLI commands share it:

- **`check` (`src/dsl/`)** — lex → parse → load project → resolve →
  elaborate → validate. Turns a multi-file `.wb` project into an
  elaborated IR (`ir::Design`) and reports problems via miette.
- **`render` (`src/render/`)** — runs the same DSL pipeline, then draws
  every view in the resulting `ir::Design` to SVG (one file per view) plus
  an `index.html` (`render::index_html`) that embeds them all for browsing.
  The legacy YAML model/view loader has been removed; `ir::Design` is the
  only thing the renderer consumes.

## DSL mental model

- **AST** — a faithful parse of one `.wb` file. `Definition` (a component
  *type*) holds `Port`/`Connector`/`Instance`/`Wire`/nested-`Definition`
  members; `View`s are top-level siblings. Every node carries a `Span`;
  type/instance/port references are *unresolved* `Spanned<Ident>`.
- **Resolved registry** — every definition (top-level and nested) keyed by
  `DefId`, with flattened ports (connectors are grouping metadata, not a
  namespace — port names are unique per component), per-file type scopes
  (own defs + `use` imports), and resolved instance/endpoint/include refs.
- **IR (`ir::Design`)** — the elaboration: a flat
  `IndexMap<InstancePath, Instance>` (hierarchical semantics, no recursive
  ownership; the tree lives in `children` links). One node per placement,
  addressed by a dotted path (`vehicle.front.module_1.pack`), with
  materialized ports and wires rewritten to `WireEnd::Own`/`Child`.
  Definitions vanish here; only concrete instances flow to the IR.

## Render mental model (IR → SVG)

The renderer consumes `ir::Design` directly — there is no separate model.
`render::render_views` walks `design.views`; each view documents a
component *type* and is rendered against the first instance of that type
(the root for a top-level view). The subject instance's **direct children**
are the includable boxes; the subject's own **wires** are the connections.

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
  (`[a,b,c]` → `a–b, b–c`). A pair is drawn only when both ends are
  `WireEnd::Child` on *listed* ports of *included* instances; `Own` ends,
  excluded instances, and unlisted ports drop silently. So a listed port
  whose wire lands on an unlisted/own end shows as a bare stub.

Box geometry is unchanged from before, minus author-supplied sizes: a
view's `grid:` step (world units; `DEFAULT_GRID` when omitted), `include`
`x`/`y` is the box **centre** in **grid units** (renderer multiplies by the
step). Ports sit a fixed **two steps** apart, **centred** on each side
(even count straddles the centreline, odd puts the middle port on it). A
box is always an even number of steps, so its centre and every port land on
a grid line. The side margin (corner to first port) is a full pitch (two
steps). Boxes always size from the busiest side's port count — width also
respects a text minimum (`MIN_WIDTH`); there is no explicit `width`/`height`
in the DSL. The routing clearance and nudge gap are one grid step, so wire
bundles stay grid-integral; routing otherwise sees only world geometry.
Because the pitch is two steps, the grid must be at least
`MIN_PORT_PITCH / 2`; a finer grid errors.

## DSL validation (`check`)

Problems are miette `Diagnostic`s (`dsl::diagnostics::Problem`), collected
so one run reports many. Errors fail the run; warnings fail only under
`--strict`. The checks, by phase:

- **Load** — file not found for a `use`; no `main.wb`; IO.
- **Parse/lex** — syntax and lexical errors (with expected-token sets).
- **Resolve** — undefined type, unresolved import, duplicate
  type/instance/port, unknown instance/port in a wire endpoint,
  private-port access (a non-`pub` port referenced from outside), unknown
  view include, ambiguous view subject. View `ports { }` placements get the
  same treatment as wire endpoints: unknown side (`unknown_port_side`),
  unknown/private port, and a duplicate-port-in-one-include guard
  (`duplicate_view_port`).
- **Elaborate** — `main.wb` lacks a single top-level component (no root);
  containment cycle (a component instantiating itself transitively).
- **Validate** — wire arity (fewer than two endpoints, error); unused
  import and bare-port pin (warnings).

Not done on purpose: **unconnected-port** detection. It needs per-instance
tree analysis and floods intentional unused-pin warnings on a real
component library — a separate, opt-in concern. See `dsl/validate/mod.rs`.

## Render-time errors

Reference and structural checks happen in `check` (the Resolve/Validate
phases above). Render adds only geometry/dispatch errors, in the slim
`error::Error` enum (`src/error.rs` — render-path only; DSL problems are
miette `Diagnostic`s):

- an unknown view `kind:` (only `schematic` today);
- a view subject type with no instance in the design;
- a non-positive `grid:`, or a `grid:` finer than a port label needs;
- file IO when writing the SVGs.

`render` runs `check_project` first and refuses to render a project that
has errors (or, under `--strict`, warnings).

## Out of scope for the MVP — resist drift

These land later, one at a time. Don't pre-bake hooks for them; we'll
redesign each when it lands.

- Harness/Graphviz renderer, BOM views, manifest emission
- View composition / `extends`
- Theming, colour
- Auto-layout
- Non-rectangle component symbols
- Visual grouping of ports by connector on a side (bracket + label)
- Per-port styling (input/output, voltage class, gauge, etc.)

## Architecture

```
src/
├── main.rs          # clap CLI: `check` and `render` (both over the .wb DSL)
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
│   ├── project/mod.rs   # walk-up discovery + transitive `use` loading → Project
│   ├── resolve/mod.rs   # DefId registry, scopes, flattened ports, reference checks
│   ├── elaborate/mod.rs # AST/registry → ir::Design; containment-cycle guard
│   ├── ir/mod.rs        # id newtypes + elaborated Design/Instance/Port/Wire/View
│   ├── validate/mod.rs  # wire arity (error) + --strict warnings
│   └── diagnostics/mod.rs # miette `Problem` enum (one variant per failure class)
│
│  # ── ir::Design → SVG renderer ──
├── render/
│   ├── mod.rs       # render_views: subject lookup + per-view dispatch + slug
│   ├── geometry.rs  # Point; re-exports ir::Side (sides are authored)
│   └── schematic/   # rectangle-based SVG renderer
│       ├── mod.rs       # SchematicRenderer; render orchestration
│       ├── layout.rs    # Placement: derive sides + boxes/ports in world coords
│       ├── draw.rs      # SVG emission (named `draw`, not `svg`, to
│       │                #   avoid clashing with the `svg` crate)
│       └── route/       # orthogonal connector routing (paper §4–6)
│           ├── mod.rs       # Router: build OVG once, route_all + nudge
│           ├── geometry.rs  # Rect, Dir
│           ├── visibility.rs# orthogonal visibility graph (§4)
│           ├── astar.rs     # A* via the `pathfinding` crate (§5)
│           └── nudge/       # separate wires sharing a channel (§6)
│               ├── mod.rs       # pipeline: segments → order → place
│               ├── segments.rs  # maximal segments + shared-edge detection
│               ├── order.rs     # §6.1 order routes within a channel
│               ├── place.rs     # §6.2 final placement (two axis passes)
│               └── vpsc.rs      # separation-constraint solver
└── error.rs         # thiserror types (render path)
```

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
- [`miette`] (feature `fancy`) — `Diagnostic` derives plus the pretty
  terminal renderer for `check` (`--format json` uses `JSONReportHandler`).
- [`indexmap`] — order-preserving maps (DSL registry/IR).
- [`clap`] (derive) — CLI parsing.
- [`thiserror`] — typed library error enums; underpins the `Diagnostic`s.
- [`anyhow`] — error glue in `main` only.
- [`svg`] — SVG document emission with escaping handled (render path).
- [`pathfinding`] — A* over the orthogonal visibility graph for
  object-avoiding connector routing (render path).

Dev / test:

- [`insta`] — snapshot tests.
- [`assert_cmd`] — black-box CLI tests.
- [`predicates`] — assertions for `assert_cmd`.

[`chumsky`]: https://docs.rs/chumsky
[`miette`]: https://docs.rs/miette
[`indexmap`]: https://docs.rs/indexmap
[`clap`]: https://docs.rs/clap
[`svg`]: https://docs.rs/svg
[`pathfinding`]: https://docs.rs/pathfinding
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
cargo run -- check examples/main.wb        # or just `check` from inside the project
cargo run -- check --strict --format json examples/main.wb

# render every view in a .wb project to SVG (one file per view, into --out)
cargo run --release -- render examples/main.wb --out out/
```

## Done definition for the MVP

- The render command above writes one valid SVG per view into `out/`.
- Opening an SVG shows something recognisable as a schematic: labelled
  rectangles with named ports on derived sides, pin numbers shown,
  Manhattan wires between connected ports.
- `cargo test` passes; the integration test renders the example project
  and asserts on SVG fragments (not a full snapshot).
