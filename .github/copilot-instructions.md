# Wirebug — Copilot Instructions

Text-defined electrical schematics. YAML model in, SVG out. See [CLAUDE.md](../CLAUDE.md) for full architecture and design decisions.

## Build & Test

```sh
cargo build
cargo test
cargo fmt
cargo clippy -- -D warnings
cargo run --release -- render --model examples/model.yaml --view examples/views/hv_overview.yaml --out hv.svg
```

Run `cargo fmt && cargo clippy -- -D warnings && cargo test` before committing.

## Architecture

```
src/
├── main.rs          # clap CLI; thin shim over render_paths
├── lib.rs           # re-exports + render_paths orchestration
├── model.rs         # Model, Component, Connector, Port, Connection
├── view.rs          # View, ViewKind
├── render/
│   ├── mod.rs       # Renderer trait; dispatch on ViewKind
│   └── schematic/   # rectangle-based SVG renderer
│       ├── mod.rs       # SchematicRenderer; render orchestration
│       ├── layout.rs    # Placement: boxes + ports in world coords
│       ├── draw.rs      # SVG emission
│       └── route/       # orthogonal connector routing
└── error.rs         # thiserror types
```

- **Model** = facts about the physical system. Never describes drawing position.
- **View** = renderable subset with layout. Owns renderer choice, positions, port placement.
- Parsing lives on the types: `Model::load(path)` / `View::load(path)`, `text.parse::<Model>()` via `FromStr`.

## Key Conventions

- **Rust 2024 edition**, stable toolchain.
- Order-preserving maps (`IndexMap`) — YAML order is meaningful.
- Port refs in connections are three-part: `component.connector.port`. Within a view's ports block, two-part: `connector.port`.
- Newtypes for IDs (`ComponentId`, `ConnectorId`, `PortId`, `Pin`). Never pass raw strings where a typed ID belongs.
- Behavior lives on types (methods/trait impls), not free functions.
- `thiserror` in the library; `anyhow` only in `main`.
- No `.unwrap()` / `.expect()` outside tests. No `.clone()` to dodge borrows.
- `pub(crate)` for cross-module API. `pub` only at the crate boundary.
- Snapshot tests with `insta` for stable text output. Don't snapshot raw SVG — assert fragments or structural representations.
- Test names describe behavior: `connection_to_missing_port_errors`, not `test_validate_returns_err`.
- A bug fix lands with the test that would have caught it.
- Comments explain *why*, not *what*. No dead code, no commented-out code.
- Add dependencies with `cargo add <crate>`.

## Out of Scope (MVP)

Do not implement: harness/Graphviz renderer, BOM views, view composition/extends, theming, auto-layout, non-rectangle symbols, per-port styling. See [CLAUDE.md](../CLAUDE.md) for full list.
