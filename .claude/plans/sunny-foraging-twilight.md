# Whole-project code review — wirebug

## Context

A full read-through of `wirebug` (11.5k lines of Rust across the DSL pipeline,
SVG renderers, and dev server) looking for bugs, inconsistencies,
non-idiomatic Rust, and structural/readability issues.

**Headline: the codebase is in very good shape.** `cargo clippy --all-targets
-- -D warnings` is clean, tests are thorough (unit tests beside every feature,
snapshot + black-box CLI coverage), the type system is used well (newtypes,
closed enums, `WireEnd`/`Side`/`ViewKind`), errors are concrete enums with
spans, and module boundaries are sharp and documented. Nearly everything in
`CLAUDE.md`'s coding standards is actually upheld in the code.

The findings below are therefore small and targeted — there is no
architectural rework to do. They are grouped by severity. Each names the file
and the recommended change. None require touching the public API except the
dead-code removal.

## Scope (confirmed): action all findings

All findings are in scope. Suggested order, smallest-risk first:

1. **C1, C2** — in-code doc fixes (`main.rs`, `dsl/mod.rs`, `ir/mod.rs`).
   Trivial, no behavior change.
2. **B1, B2** — delete `Error::Io` and `trim_float`; the compiler proves
   nothing referenced them.
3. **A1** — fix the filename collision + add the regression test.
4. **C3, C4** — refresh `CLAUDE.md` (deps, tree, render bullet) and `README.md`
   (intro/feature list/View section for serve + harness + png/embed).
5. **D1** — split `resolve/mod.rs`: move view-resolution
   (`check_view_ports`, `check_enclosure`, `check_*_include`,
   `check_duplicate_includes`, `resolve_views`, `include_target`) into a new
   `dsl/resolve/views.rs` submodule; keep the registry passes in `mod.rs`.
   Pure code move — no logic change, so the existing resolve tests are the
   safety net. Do this last since it touches the most lines.

D2/D3/E are noted but optional polish; fold them in opportunistically while
touching `resolve/mod.rs` for D1 if convenient.

---

## Findings

### A. Correctness bugs (worth fixing)

**A1 — `RenderedView` filename collision across slug bases.**
`render/mod.rs::FilenameAllocator` keys its `seen` counter on the *base* slug
only, so a disambiguated name can collide with a genuine title:
- `"Overview"` → `overview.svg`
- `"Overview"` → `overview_2.svg`
- `"Overview 2"` → base `overview_2`, count 1 → `overview_2.svg` ← **collides**

In SVG mode the second file silently overwrites the first on disk, and the
HTML index points two views at one image. Fix: track the set of *final emitted
names* and bump the suffix until the formatted name is unique (not just the
base). Add a regression test alongside
`filename_allocator_disambiguates_duplicate_titles` covering the
`"X"`, `"X"`, `"X 2"` sequence.

### B. Dead code

**B1 — `error::Error::Io` is never constructed.** Confirmed by grep: nothing
in the crate builds `Error::Io`. The render path uses `Error::Write` for
output; source reads surface as `diagnostics::Problem::Io` in the DSL path.
Remove the variant from `src/error.rs` (CLAUDE.md says "No dead code"). Its doc
comment ("Failed to read a file from disk") also overlaps confusingly with
`Write`.

**B2 — `harness/draw.rs::trim_float` duplicates std `f64` Display.**
`format!("{}", 50.0_f64)` already yields `"50"` and `format!("{}", 0.25)`
yields `"0.25"` — Rust's shortest-round-trip formatting already drops the
trailing `.0`. The helper's `v as i64` branch is not only redundant but worse
for large values (truncation/overflow). Replace
`wire_annotation`'s `trim_float(gauge)` with `format!("{gauge}mm²")` directly
and delete `trim_float`. (Cable length in `harness/layout.rs::cable_subtitle`
already relies on plain `{l}` formatting — so this also removes a small
inconsistency between the two.)

### C. Documentation drift (inconsistencies)

These are "comments must stay true" issues — the code moved on, the prose
didn't.

**C1 — "Two subcommands" in `main.rs:3`.** There are three: `check`, `render`,
`serve`. Update the module doc.

**C2 — Stale "later change" scaffolding comments:**
- `dsl/ir/mod.rs:5` — "The elaborated `Design` … lands in a later change; for
  now this module defines the names." `Design` is fully defined right below.
- `dsl/mod.rs:10` — "resolution, elaboration, and validation land in later
  changes." They have landed; the pipeline runs all of them.
- `dsl/mod.rs:39` — "the problems found and (later) the elaborated IR." Drop
  the "(later)".

**C3 — `CLAUDE.md` dependency list and architecture tree are behind the code.**
The runtime-deps list omits `resvg` (PNG rasterisation), `serde_json` (embed
manifest); the dev-deps list omits `tempfile`; the `src/` tree omits
`render/png.rs`. The `render` bullet still says only SVG/`index.html` and
doesn't mention `--png` / `--embed`/`manifest.json`. Add these so the file
keeps earning its "read this for architecture" role.

**C4 — `README.md` is materially staler than CLAUDE.md.** It describes the
renderer as schematic-only ("a kind (`schematic` for now)", line 95) and the
feature list (lines 19–26) omits `serve`, the harness renderer, and PNG output.
The embed/manifest section (line 118) *is* current, so the file is
half-updated. Recommend a pass to bring the intro and "View" section in line
with the harness + serve + png reality. (Lower priority than C1–C3 since it's
user-facing prose, but it's the most out-of-date doc in the repo.)

### D. Readability / structure (optional)

**D1 — `dsl/resolve/mod.rs` is the one oversized module** (~800 lines before
tests). It carries two distinct concerns: the definition/endpoint registry
(passes 1–2) and *view* validation (`check_view_ports`, `check_enclosure`,
`check_*_include`, `resolve_views`). Consider splitting view-resolution into a
`resolve/views.rs` submodule. Not urgent — the file is cohesive and
well-commented — but it's the only file that's genuinely big once tests are
excluded. Everything else is comfortably reviewable.

**D2 — `Resolver::register` double-initialises each `DefInfo`.** It pushes a
`DefInfo` with empty `ports`/`instances`/`nested` to reserve the `DefId`
before recursing, then rebuilds those three maps locally and assigns them back
(lines ~130–138 and ~215–217). The reservation is necessary (children need the
parent id mid-recursion), but the empty-then-real pattern reads awkwardly. A
small comment explaining *why* the slot is reserved first, or restructuring to
build the maps before the recursive `register` calls, would help the next
reader. Minor.

**D3 — `register`'s connector-name destructure is redundant.**
`if let (Some(n), Some(named)) = (name, conn.name.as_ref())` derives both
halves from `conn.name`; it can collapse to `if let Some(named) = &conn.name`
with `let n = named.node.as_str();`. Cosmetic.

### E. Minor notes (no action needed unless convenient)

- **E1 — Negative coordinates are unrepresentable.** The lexer has no sign
  handling, so `include`/`enclosure`/`text` coordinates and wire gauges can't
  be negative. This appears intentional (layout auto-fits, so non-negative
  grid space loses nothing), but it's an implicit constraint worth a one-line
  note in the DSL/lexer docs if it isn't already covered by the skill spec.
- **E2 — `schematic` `MIN_HEIGHT` naming.** It's only used for the empty-view
  viewbox fallback; real boxes have no height floor (`box_dimensions` says so).
  The name suggests a per-box minimum it doesn't enforce. Harmless.

---

## What is *not* a problem (checked and sound)

- The routing stack (OVG → A* → §6 nudge/VPSC) is faithful to the cited paper,
  the VPSC merge loop provably terminates (≤ n−1 merges), and obstacle/edge
  gating is correct. Good test coverage including detour regressions.
- Error handling: no `unwrap`/`expect` outside tests; `anyhow` confined to
  `main`; miette `Diagnostic`s carry spans + source.
- The trivia-preserving lexer + `significant()` "dial" is a clean seam for a
  future `fmt`.
- Loader de-dups by canonical path and reports each file once (diamond-import
  test confirms).
- `serve` shutdown handling (pinned `notified()` across loop iterations in both
  the watcher and the websocket handler) correctly avoids the
  notify-while-busy race.

---

## Verification

After applying any subset of fixes:

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings   # must stay clean
cargo test                                   # unit + snapshot + CLI
```

- **A1**: new unit test in `render/mod.rs` for the `"X"/"X"/"X 2"` collision;
  assert the three filenames are pairwise distinct.
- **B1/B2**: deletions — `cargo build` proves nothing referenced them; existing
  `wire_annotation` test (`annotation_combines_label_and_gauge`) still passes
  for B2.
- **C1–C4**: doc-only; `cargo test` (doctests) + a re-read confirm accuracy.
- End-to-end sanity: `cargo run -- render examples/main.wb --out /tmp/wb` and
  open `/tmp/wb/index.html`; `cargo run -- serve examples/main.wb` and confirm
  live reload still works.
