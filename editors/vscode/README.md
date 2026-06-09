# Wirebug for Visual Studio Code

Language support for the wirebug `.wb` DSL: syntax highlighting today, with a
language server (live diagnostics + completion, via `wirebug lsp`) landing in
later phases.

## Try it (development)

Open this folder (`editors/vscode/`) in VSCode and press **F5**. An Extension
Development Host opens on the repo's `examples/` project — open any `.wb` file
to see the highlighting.

## Install locally

```sh
npm install
npm run package                 # builds wirebug-<version>.vsix
code --install-extension wirebug-*.vsix
```

(The `package` script arrives with the language-client phase; until then the
extension is grammar-only and F5 is the way to run it.)
