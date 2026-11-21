# wirebug — working notes

Text-defined electrical schematics and wiring harnesses: a multi-file `.wb`
DSL project, rooted at a `wirebug.toml` manifest, is checked and rendered to
SVG/PNG/PDF. `README.md` is the user-facing intro; the language itself is
specified in `.claude/skills/wirebug-dsl/`.

This file holds the rules for working on the codebase. **How the application
works lives in `ARCHITECTURE.md`** — read the section for the area you are
about to change before changing it, and update it in the same change.

## Orientation

One pipeline, shared by every command (`check`, `render`, `serve`, `lsp`):

```
.wb files → lex → parse (AST) → load project → resolve (registry)
          → elaborate (ir::Design) → validate → layout checks
          → render (SVG per view)
```

- `src/dsl/` — everything up to and including `validate`; problems are
  miette `Diagnostic`s (`dsl::diagnostics::Problem`).
- `src/pipeline.rs` — `check_loaded`, the one place the stages after
  loading are sequenced (the `dsl` stages, then the layout checks in
  `src/render/lint.rs`, which ask the renderers' layout code and so sit in
  `render`; `dsl` never depends on `render`). `check_project` and the LSP
  both go through it; a new stage goes there.
- `src/export/` — build tables derived from `ir::Design` (the wire list),
  typed models with no rendering involved.
- `src/render/` — consumes `ir::Design` only; one renderer per view kind
  (`schematic/`, `harness/`, `pinout/`) plus `png.rs`, `pdf.rs`, embed output.
- `src/serve/` — live-reloading dev server (the only async code).
- `src/lsp/` — language server over stdio; `editors/vscode/` is its client.
- `tests/` — integration tests against the projects in `tests/fixtures/`.
- `examples/` — the real vehicle project; must keep working, never tested against.

Where to look in `ARCHITECTURE.md`:

| Working on | Section |
|---|---|
| CLI commands, output modes, `serve`, `lsp` | One pipeline |
| `wirebug.toml`, the identity stamp | Project manifest |
| AST, registry, `extend` merge groups, IR, cables, inline connectors | DSL mental model |
| Schematic layout, routing, enclosures | Render mental model; Connector routing |
| Harness views | Harness views |
| Wire list and other build tables (`src/export/`) | Build tables |
| A new or changed diagnostic, and which phase it belongs in | DSL validation; Render-time errors |
| Finding a file | Source tree |
| Why a crate is there | Dependencies |

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
- More than the basic `splice` member (landed: a named joint that
  two-ended conductors end on, placed and drawn in harness views, drawn as
  one net in schematics): splices in schematic or pinout includes, shield
  drains on a splice, junction dots, multi-endpoint wires through a splice.
  Shared rails stay valid loose multi-endpoint wires; `cable` conductors
  stay point-to-point.
- Port re-export aliases (`pub port x "X" = inner.port;` — a pure boundary
  mapping with no wire). Decided against for now (2026-06): the wire-through
  pattern stays; revisit when a BOM view makes invented color/gauge on
  zero-length boundary wires actually hurt.

## Coding practices

### Testing — lock in behavior

- Every public feature has unit tests alongside the code
  (`#[cfg(test)] mod tests`); end-to-end pipeline and CLI tests run
  against the small fixture projects in `tests/fixtures/`.
- **Never test against `examples/`** — it is the real, freely-changing
  vehicle project. When a feature needs end-to-end coverage, extend a
  fixture (or add one) instead.
- Test names describe behavior, not implementation:
  `connection_to_missing_port_errors`, not `test_validate_returns_err`.
- Snapshot tests with `insta` for stable text output: AST `Debug`,
  CLI help, error messages, normalised structural views of an SVG. Review
  with `cargo insta review`.
- **Don't snapshot raw SVG strings** — layout pixels churn on every
  renderer tweak. Either assert on fragments (presence of expected
  `<rect>`s, port labels, wire endpoints) or snapshot a derived
  structural representation.
- CLI tests with `assert_cmd`.
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
- Every major chumsky production is `.boxed()` where it's defined, and an
  unboxed heavyweight parser is never embedded twice — otherwise rustc's
  typeck cost explodes (builds have gone from seconds to OOM).

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
- SVG emission goes through the `svg` crate. It handles XML escaping
  of user-supplied labels (a small but real foot-gun if hand-rolled)
  and gives a discoverable element-builder API. We still own structure,
  classes, and embedded `<style>`.

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
cargo run --release -- render examples --out out/ --pdf    # single A4 PDF, one view per page
cargo run --release -- render examples --out out/ --embed  # naked SVGs + manifest.json

# serve a project with live reload (re-renders on every .wb save)
cargo run -- serve examples --port 3000   # then open http://localhost:3000

# the language server runs over stdio — editors spawn it, not humans
cargo run -- lsp

# build + install the VSCode extension (see editors/vscode/README.md)
cd editors/vscode && npm install && npm run package
code --install-extension wirebug-*.vsix
```

## Releasing

Releases are **tag-driven**: pushing a `vX.Y.Z` tag runs
`.github/workflows/release.yml`, which gates on the shared
`quality-checks.yml` (the same reusable job `ci.yml` runs on PRs, with the
font install the render tests need), then, in order:

- **build** the binary for `x86_64-unknown-linux-gnu`,
  `x86_64-apple-darwin`, and `aarch64-apple-darwin` (no Windows);
- **vsix** — build the VSCode extension into `wirebug-<version>.vsix`;
- **release** — a GitHub release whose notes are generated by `git-cliff`
  from the conventional-commit history (default config, no `cliff.toml`),
  with the three binaries and the `.vsix` attached;
- **homebrew** — render `Formula/wirebug.rb` into `LHelge/homebrew-tap`
  (needs the `TAP_GITHUB_TOKEN` secret, a PAT with write access to the tap);
- **publish** — `cargo publish` to crates.io (needs `CARGO_REGISTRY_TOKEN`).
  This is **irreversible**: a version can only be yanked, never replaced.

**Before tagging, bump the version in two places by hand to match the tag**
(the `vX.Y.Z` tag → `X.Y.Z`, no `v`):

1. `Cargo.toml` `[package] version` — this is what `cargo publish` uploads,
   so a mismatch means the crate version differs from the tag. Verify with
   `wirebug manifest version` (prints `vX.Y.Z`).
2. `editors/vscode/package.json` `version`.

There is **no automatic sync** between these and the tag. CI does re-stamp
the `.vsix` from the tag (`vsce package "$VERSION"`), so the *released*
extension always matches even if `package.json` drifts — but keep
`package.json` correct anyway so local `npm run package` builds and the
committed source agree with the release. Cargo.toml has no such safety net:
its committed version is published verbatim, so it **must** be bumped.

Conventional commits drive the changelog, so keep using `feat:` / `fix:` /
etc. scopes. The commit that bumps the versions is the natural release
commit to tag.

## Done definition for the MVP

- The render command above writes one valid SVG per view into `out/`.
- Opening an SVG shows something recognisable as a schematic: labelled
  rectangles with named ports on derived sides, pin numbers shown,
  Manhattan wires between connected ports.
- `cargo test` passes; the integration tests render the fixture projects
  and assert on SVG fragments (not a full snapshot).
