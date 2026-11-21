---
name: implement-epic
description: Implement a whole Bears epic unattended, from its planned tasks to a pull request with green CI, by orchestrating one task-implementer subagent per task and a task-reviewer for each.
argument-hint: <epic-id>
disable-model-invocation: true
---

Implement the Bears epic `$ARGUMENTS` from start to a pull request with green CI, without
the user watching. They read one thing at the end: the pull request and your final report.

You are the orchestrator. You write no feature code and read no full diffs: each task goes
to a `task-implementer` subagent and each diff to a `task-reviewer` subagent, so that your
context lasts for the whole epic. What only you can do is the part between the tasks:
choosing the order, telling each agent what the others built, deciding on review findings,
integrating the branches, and judging whether the epic as a whole is done.

Use the installed `bea` CLI for all task tracking during the run, not the Bears MCP tools:
the MCP server writes to the main checkout whichever directory an agent works in.

## Before starting

Stop and tell the user, instead of working around it, if any of these fails:

- `bea show <epic> --json` is an epic with planned sub-tasks (`bea list --epic <epic> -a --json`).
- Every epic or task in the epic's own `depends_on` is done. `bea ready --epic` does not
  check this, so check it yourself.
- The working tree is clean, and `main` is up to date with `origin/main`.
- `cargo test` passes on `main`.

Then read the epic's body, every task's body and dependencies (`bea show <id> --json` for
each; the list output leaves the dependencies out), `AGENTS.md`, and the sections of
`ARCHITECTURE.md` that the epic touches. You need to know the epic well enough to tell a correct seam from a wrong
one. Do not read the code the tasks will change.

Create the epic's branch from `main`: `<type>/<short-slug>-<epic-id>`, for example
`feat/harden-check-kss`. If the branch already exists, this is a resumed run: check it
out, see what is committed, `bea release` any task left in progress with no commits, and
continue with the loop.

## The loop

Repeat until `bea ready --epic <epic> --json` returns nothing and no agent is running.

A task tagged `manual` is the user's to do: never give it to an implementer. Tasks of this
epic can depend on tasks of other epics, and those you do not implement either. So the
loop can end with tasks still open: blocked by a manual task, by a task outside the epic,
or by a task you released. That is a partial run; see "Finishing the epic".

### 1. Choose the batch

Take the ready tasks. Run them one at a time unless running some together is safe. This is
one crate with one `target/` directory, and most tasks meet in the same shared files: the
`Problem` enum (`src/dsl/diagnostics/mod.rs`), the IR (`src/dsl/ir/mod.rs`), the AST and
parser, the insta snapshots, `Cargo.lock`, and the documentation (`ARCHITECTURE.md`,
`README.md`, the `wirebug-dsl` skill):

- Two tasks run together only when their bodies make it clear that they touch different
  modules: for example a docs, CI-workflow or `editors/vscode/` task beside a Rust task,
  or two renderers (`render/schematic/`, `render/harness/`, `render/pinout/`) or frontends
  (`serve/`, `lsp/`) that share no DSL, IR or diagnostics change. Anything that changes
  the `.wb` syntax runs alone: it reaches the parser, the fixtures, the examples, the
  TextMate grammar and every doc. If you cannot tell, run them in sequence.
- No more than two agents at once.

A single task, and the first task of a parallel batch, works in the main checkout on the
epic's branch. The other task of a batch gets its own worktree: spawn it with
`isolation: "worktree"`. The project's settings base such a worktree on the current HEAD,
so the main checkout must be clean and committed when you spawn it. A build in a fresh
worktree starts from nothing.

### 2. Spawn the implementers

One `task-implementer` per task, all of a batch in the same message. Its prompt holds:

- the task id and the epic id (it reads the bodies itself; do not paste them);
- where it works: the main checkout on the epic's branch, or its worktree;
- the seams it builds on, from earlier implementers' reports: the types, functions,
  modules, IR fields, diagnostics, syntax or CLI flags that exist now and where they are;
- decisions already made in this epic that bind it;
- for a batch: which other task runs beside it and what it owns, so that it keeps out.

### 3. When an implementer reports

Check the report against the repository, briefly: the commits exist
(`git log --oneline <base>..<head>`), the tree is clean (no stray `*.snap.new`, no render
output), `git diff --stat <base>..<head>` touches what the task is about. Do not read the
diff.

If it stopped without finishing, decide: answer it with SendMessage if you can resolve what
blocked it from the epic or `AGENTS.md`; otherwise `bea release <id>`, note it for the
final report, and go on with the tasks that do not depend on it.

Otherwise spawn a `task-reviewer` with the task id, the directory and the commit range, and
nothing else. It must not see the implementer's report: a reviewer handed the author's
account tends to confirm it.

### 4. When a reviewer reports

Decide on every finding yourself; the reviewer advises.

- Blocking and should-fix findings you agree with go back to the same implementer with
  SendMessage, so that it fixes them with its context intact. Send the findings as the
  reviewer wrote them, with the `file:line` and the failing case.
- A finding you judge wrong, or outside the task, you drop, with the reason kept for the
  final report. A finding that is real but belongs to another task becomes a Bears task
  (`bea create … --parent <epic>`, with `bea dep add` where it blocks something).
- After fixes, send the reviewer the new range with SendMessage for a second look at just
  those findings. Two rounds at most: what is still open after the second becomes a Bears
  task under the epic, and you move on.

### 5. Integrate

A task done in the main checkout is already on the epic's branch. For a worktree task:

1. `git -C <worktree> rebase <epic-branch>`. A conflict only in `Cargo.lock` is resolved by
   taking the epic branch's version and running `cargo build`. Any other conflict: abort
   the rebase and send it to the task's implementer, which knows both the code and its
   worktree.
2. `git merge --ff-only <task-branch>` in the main checkout. Never a merge commit.
3. `git worktree remove <worktree>` and delete the task's branch.

After a rebased task is integrated, run the checks on the epic's branch:

```sh
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo run -- check --strict examples
cargo run -- render examples --out <a directory in your scratchpad>
```

CI runs all three. The last two are not tests (nothing asserts on `examples/`), but that
directory is the generic example project, and CI fails on any error or warning `check
--strict` reports for it, or on a view that does not render.
Send the output to a file in your scratchpad and read only the end of it. A failure goes
to the implementer whose task it concerns.

Discovered work from the implementers' reports becomes Bears tasks now: under the epic if
the epic is not done without it, otherwise on its own with the right priority. Commit new
and changed `.bears/` files that no implementer committed
(`chore: track work found in the <name> epic (<epic>)`).

## Finishing the epic

When the loop has ended, with every task done or with the rest blocked (a partial run):

1. Run the checks above on the epic's branch, output to files. Fix failures through the
   implementer concerned.
2. Spawn a `task-reviewer` for an epic review of `main..HEAD`. Handle its findings as in
   step 4, sending each to the implementer of the task it concerns.
3. Read the seams yourself, and only those: the diff of `src/lib.rs`, `src/main.rs`,
   `Cargo.toml`, `src/dsl/ir/mod.rs`, `src/dsl/diagnostics/mod.rs`, `AGENTS.md`,
   `ARCHITECTURE.md`, `README.md` and `skill/SKILL.md` (the skill's source). Check them against the epic's
   body.
4. `bea done <epic>` if it did not auto-close, and commit it:
   `chore: complete the <name> epic (<epic>)`. In a partial run, leave the epic open and
   skip this step; the pull request's title and description say that it is a part, and list
   each open task with what blocks it.
5. Rebase on `origin/main` if it moved, push, and open the pull request with
   `gh pr create`. Its description, written for a reader who has not followed the run:
   what the epic delivers, in the user's terms; the tasks with their ids; **breaking
   changes**, each with what the author of a `.wb` project has to do; **try it**: the
   commands to run and what to look at for everything the tests cannot judge (how a
   drawing reads, how the editor behaves), since the user tests those by hand before
   merging; **decisions made**: every place where an implementer or you chose between
   readings, with the reason, so the user can overrule one cheaply; findings you dropped
   and why; tasks you created; anything not done.
6. Watch CI (`gh pr checks <number> --watch`). A failing job is fixed through the
   implementer concerned and pushed, until "fmt · clippy · test" is green.

Do not merge the pull request. Never bump the version in `Cargo.toml` or
`editors/vscode/package.json`, create a release tag or publish, even when a task asks for
a release: a tag publishes to crates.io, which cannot be undone. Leave that task open and
report it.

## Deciding without the user

The user is not there, so do not stop to ask what you can decide. Where a task is open to
two readings, take the one that fits the epic's body and `AGENTS.md`, and record it under
"decisions made". Prefer the reading that is easier to change later.

Stop and report, with the epic's branch left as it is, only when going on would mean one
of these: a breaking change to the `.wb` syntax, the `wirebug.toml` manifest, the CLI or
the `--embed` output (`manifest.json`, the SVG classes hosts style) that the epic does not
call for; building something from the "Out of scope for the MVP" list of `AGENTS.md` that
the epic does not call for; rewriting `examples/` beyond what a change in the epic
requires; a change to the repository's `.bears/` files beyond normal task tracking;
deleting or rewriting work outside the epic; a task that contradicts `AGENTS.md`; or the
same task failing after two implementers have tried it and nothing else is ready.

## The final report

A few lines: the pull request's link and whether CI is green; the tasks done and any
released; the decisions the user should look at first; what to try by hand; the Bears
tasks you created. If the run stopped early, lead with what stopped it and what the user
needs to decide.
