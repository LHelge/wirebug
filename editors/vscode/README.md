# Wirebug for Visual Studio Code

Language support for the [wirebug](https://github.com/LHelge/wirebug) `.wb`
DSL: syntax highlighting, live diagnostics, and context-aware code
completion via the `wirebug lsp` language server.

## Requirements

The extension does not bundle the language server — it spawns the
`wirebug` CLI, which must be installed separately:

```sh
cargo install --path .          # from a checkout of the wirebug repo
```

See the [wirebug repository](https://github.com/LHelge/wirebug) for build
and install instructions. Syntax highlighting works without the binary;
diagnostics and completion need it.

The server binary is resolved in this order:

1. the `wirebug.server.path` setting (an absolute path to the binary);
2. the newest build in the repo's `target/` — only when the extension
   runs from source (F5, see Development below);
3. `wirebug` on `PATH` — the normal case for an installed extension.

If the server fails to start, install wirebug or point
`wirebug.server.path` at a binary, then run **Wirebug: Restart Language
Server** from the command palette. The same command picks up a replaced
binary after a rebuild.

## Installing the extension

Not on the marketplace yet — package the `.vsix` and install it:

```sh
cd editors/vscode
npm install
npm run package                 # typecheck + bundle + wirebug-<version>.vsix
code --install-extension wirebug-*.vsix
```

## Settings

- `wirebug.server.path` — path to the `wirebug` binary used as the
  language server. Leave empty to resolve as described above.
- `wirebug.trace.server` — log the LSP traffic to the Output panel
  (`off` / `messages` / `verbose`).

## Development

```sh
cargo build                     # from the repo root — builds the language server
cd editors/vscode
npm install
```

Then open this folder (`editors/vscode/`) in VSCode and press **F5**. An
Extension Development Host opens on the repo's `examples/` project; the
client picks up the freshest `target/{debug,release}/wirebug` build
automatically. Break a wire endpoint and the squiggle appears on save-less
keystrokes.

Note that a development-host extension never shows up in the Extensions
view (only installed extensions do) — confirm it's running with
**Developer: Show Running Extensions**, or just open a `.wb` file and
check the language mode in the status bar.
