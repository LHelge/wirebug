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
 */
function findServer(context: vscode.ExtensionContext): string {
  const configured = vscode.workspace
    .getConfiguration("wirebug")
    .get<string>("server.path");
  if (configured) {
    return configured;
  }
  for (const profile of ["release", "debug"]) {
    const candidate = context.asAbsolutePath(
      path.join("..", "..", "target", profile, "wirebug"),
    );
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return "wirebug";
}

export async function activate(context: vscode.ExtensionContext) {
  const serverOptions: ServerOptions = {
    command: findServer(context),
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

  await client.start();
}

export async function deactivate() {
  await client?.stop();
  client = undefined;
}
