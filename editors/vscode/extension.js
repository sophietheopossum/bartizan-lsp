// Thin VS Code client for Bartizan.
//
// VS Code has no way to register an arbitrary language server from settings —
// that capability only exists in extensions — so this exists purely to spawn the
// binary and register it for Rust. Everything interesting happens server-side.
//
// This runs *alongside* rust-analyzer rather than instead of it. VS Code
// dispatches textDocument/codeAction to every server registered for the
// language and merges the responses, so both sets of actions share one lightbulb.

const { workspace, window } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate() {
  const configured = workspace.getConfiguration("bartizan").get("server.path");
  const command = configured && configured.length > 0 ? configured : "bartizan-lsp";

  const serverOptions = {
    command,
    args: [],
    transport: TransportKind.stdio,
  };

  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "rust" }],
    // Bartizan keeps no project model and watches no files — it only ever needs
    // the buffers the editor sends it, so there is nothing to synchronise here.
    outputChannel: window.createOutputChannel("Bartizan"),
  };

  client = new LanguageClient("bartizan", "Bartizan", serverOptions, clientOptions);
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
