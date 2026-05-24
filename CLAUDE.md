# wirebug — working notes

User-facing intro is in `README.md`. This file is for context that helps
future work on the codebase. Keep it short; don't restate the README.

## Mental model

- **Model** — facts about the physical system. `Component` → `Connector`
  → `Port`, plus `Connection`s between ports. Pin numbers, part numbers,
  labels. The model never describes *where* anything is drawn.
- **View** — a renderable subset of the model. Owns the renderer choice
  (`kind:`), title, per-component position (`layout:`), and per-component
  port placement (`ports:` — which side, in what order). Multiple views
  over one model is the whole point; resist anything that pushes
  presentation back into the model.

## Schema essentials

- Port refs in `connections:` are three-part: `component.connector.port`.
  Names cannot contain `.`.
- Within a view's `ports.{component}` block, refs are two-part:
  `connector.port` (the component is the key).
- A component is "in" a view iff it appears in `layout:`. There is no
  separate `include:` list.
- A port is "in" a view iff it's listed under one of the four sides for
  its component. Unlisted ports are silently hidden — that's the
  subsetting mechanism.
- A connection is drawn iff both endpoints are in the view.

## Validation

- **Error**: any port ref that doesn't resolve to a real
  component/connector/port (in connections or in a view).
- **Error**: duplicate component / connector / port keys.
- **Warning**: a port defined in the model that no connection touches.
  Components have terminals that a given schematic may not exercise, so
  this is a warning, not an error.
- Nothing else. "Validation beyond referential integrity" is explicitly
  out of scope for MVP.

## Out of scope for the MVP — resist drift

These land later, one at a time. Don't pre-bake hooks for them; we'll
redesign each when it lands.

- Harness/Graphviz renderer, BOM views, manifest emission
- View composition / `extends`
- Theming, colour
- Auto-layout
- Non-rectangle component symbols
- Visual grouping of ports by connector on a side (bracket + label)
- Connector *nudging* — separating wires that share a channel and
  centring them in "alleys" (paper §6). Routing already avoids
  obstacles (§4–5); nudging is the next routing increment.
- Per-port styling (input/output, voltage class, gauge, etc.)

## Architecture

```
src/
├── main.rs          # clap CLI; thin shim over render_paths
├── lib.rs           # re-exports + render_paths orchestration
├── model.rs         # Model, Component, Connector, Port, Connection;
│                    # Model::load(path) + FromStr<Model>
├── view.rs          # View, ViewKind; View::load(path) + FromStr<View>
├── render/
│   ├── mod.rs       # Renderer trait; dispatch on ViewKind
│   └── schematic/   # rectangle-based SVG renderer
│       ├── mod.rs       # SchematicRenderer; render orchestration
│       ├── layout.rs    # Placement: boxes + ports in world coords
│       ├── draw.rs      # SVG emission (named `draw`, not `svg`, to
│       │                #   avoid clashing with the `svg` crate)
│       └── route/       # orthogonal connector routing (paper §4–5)
│           ├── mod.rs       # Router: build OVG once, route per wire
│           ├── geometry.rs  # Rect, Dir
│           ├── visibility.rs# orthogonal visibility graph
│           └── astar.rs     # A* via the `pathfinding` crate
└── error.rs         # thiserror types
```

Parsing is on the types — `Model::load(path)` / `View::load(path)` for
files (errors carry the source path); `text.parse::<Model>()` via
`FromStr` for strings. There is no separate `parse` module.

## Coding practices

### Testing — lock in behavior

- Every public feature has unit tests alongside the code
  (`#[cfg(test)] mod tests`); the worked example renders end-to-end in
  `tests/`.
- Test names describe behavior, not implementation:
  `connection_to_missing_port_errors`, not `test_validate_returns_err`.
- Snapshot tests with [`insta`] for stable text output: parsed-model
  `Debug` / `serde` round-trips, CLI help, error messages, normalised
  structural views of an SVG. Review with `cargo insta review`.
- **Don't snapshot raw SVG strings** — layout pixels churn on every
  renderer tweak. Either assert on fragments (presence of expected
  `<rect>`s, port labels, wire endpoints) or snapshot a derived
  structural representation.
- CLI tests with [`assert_cmd`].
- A bug fix lands with the test that would have caught it.

### Type system — make illegal states unrepresentable

- Newtypes for "string with meaning" — `ComponentId`, `ConnectorId`,
  `PortId`, `Pin`. Cheap, and the compiler stops you mixing them.
- Enums where two fields can't both be set, or where a value has a
  closed set of variants (`Side`, `ViewKind`).
- Typestate where it earns its keep — e.g. `Model<Unvalidated>` vs
  `Model<Validated>` if the call sites benefit. Don't pre-bake it.
- Prefer std traits over bespoke helpers:
  `From` / `TryFrom`, `FromStr`, `Display`, `Debug`, `Ord` /
  `PartialOrd`, `IntoIterator`. For example, `PortRef: FromStr` parses
  `"comp.conn.port"` and `PortRef: Display` round-trips it — no
  `parse_port_ref()` free function.
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

- `thiserror` in the library; `anyhow` only in `main`.
- Library errors are concrete enums. Never `Box<dyn Error>`, never
  `Result<_, String>` — strings aren't errors.
- Each variant carries enough context to act on: a "port not found"
  error names the missing ref, the source file, and (for YAML) the
  line/column from `serde_yaml`.

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
- Order-preserving maps (`indexmap::IndexMap`) for `components`,
  `connectors`, and `ports` in the model. YAML order is meaningful as a
  default and as a tie-breaker for diagnostics.
- SVG emission goes through the [`svg`] crate. It handles XML escaping
  of user-supplied labels (a small but real foot-gun if hand-rolled)
  and gives a discoverable element-builder API. We still own structure,
  classes, and embedded `<style>`.

### Dependencies

Add with `cargo add` so versions stay current.

Runtime:

- [`serde`] (derive) — model and view (de)serialisation.
- [`serde_yml`] — YAML parser; preserves line/column on errors.
- [`indexmap`] (serde feature) — order-preserving maps for the model.
- [`clap`] (derive) — CLI parsing.
- [`svg`] — SVG document emission with escaping handled.
- [`pathfinding`] — A* over the orthogonal visibility graph for
  object-avoiding connector routing.
- [`thiserror`] — typed library error enums.
- [`anyhow`] — error glue in `main` only.

Dev / test:

- [`insta`] — snapshot tests.
- [`assert_cmd`] — black-box CLI tests.
- [`predicates`] — assertions for `assert_cmd`.

[`serde`]: https://docs.rs/serde
[`serde_yml`]: https://docs.rs/serde_yml
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
cargo run --release -- render \
  --model examples/model.yaml \
  --view  examples/views/hv_overview.yaml \
  --out   hv.svg
```

## Done definition for the MVP

- The command above produces a valid SVG.
- Opening the SVG shows something recognisable as a schematic:
  labelled rectangles with named ports on the correct sides, pin
  numbers shown, Manhattan wires between connected ports.
- `cargo test` passes; integration test renders the example and asserts
  on SVG fragments (not a full snapshot).
- Stderr shows the unconnected-port warning for `contactor.coil.*`.
