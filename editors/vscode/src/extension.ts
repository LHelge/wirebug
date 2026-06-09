import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

/**
 * Resolve the `wirebug` binary: the explicit setting wins, then a build in
 * the repo's own target/ (the extension lives at editors/vscode/ when run
 * from source, so this only fires in development), then the PATH.
 *
 * Of the repo builds, the most recently built profile wins — a stale
 * sibling (e.g. an old release build predating the `lsp` subcommand)
 * must never shadow the binary that was just compiled.
 */
function findServer(context: vscode.ExtensionContext): string {
  const configured = vscode.workspace
    .getConfiguration("wirebug")
    .get<string>("server.path");
  if (configured) {
    return configured;
  }
  const builds = ["release", "debug"]
    .map((profile) =>
      context.asAbsolutePath(path.join("..", "..", "target", profile, "wirebug")),
    )
    .filter((candidate) => fs.existsSync(candidate))
    .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  return builds[0] ?? "wirebug";
}

export async function activate(context: vscode.ExtensionContext) {
  const command = findServer(context);
  const serverOptions: ServerOptions = {
    command,
    args: ["lsp"],
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: "wirebug", scheme: "file" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/wirebug.toml"),
    },
  };
  client = new LanguageClient(
    "wirebug",
    "Wirebug Language Server",
    serverOptions,
    clientOptions,
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("wirebug.restartServer", async () => {
      await client?.restart();
    }),
  );

  try {
    await client.start();
  } catch (err) {
    void vscode.window.showErrorMessage(
      `wirebug language server failed to start (\`${command} lsp\`): ${err}. ` +
        "Check that the binary is current (`cargo build`) or set `wirebug.server.path`.",
    );
  }
}

export async function deactivate() {
  await client?.stop();
  client = undefined;
}
