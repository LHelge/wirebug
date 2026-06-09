# Wirebug for Visual Studio Code

Language support for the wirebug `.wb` DSL: syntax highlighting, live
diagnostics, and code completion via the `wirebug lsp` language server.

## Try it (development)

```sh
cargo build            # from the repo root — builds the language server
cd editors/vscode
npm install
```

Then open this folder (`editors/vscode/`) in VSCode and press **F5**. An
Extension Development Host opens on the repo's `examples/` project; the
client picks up the server from `../../target/debug/wirebug` automatically.
Break a wire endpoint and the squiggle appears on save-less keystrokes.

## Install locally

```sh
npm install
npm run package                 # typecheck + bundle + wirebug-<version>.vsix
code --install-extension wirebug-*.vsix
```

The installed extension finds the server through the `wirebug.server.path`
setting, or `wirebug` on PATH (`cargo install --path .` from the repo root).

`Wirebug: Restart Language Server` from the command palette restarts the
server after replacing the binary.
