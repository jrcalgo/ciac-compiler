// The CIaC extension's whole job: launch `ciac lsp` as a language
// server for `.ciac` files. Syntax highlighting is a static grammar
// contribution (package.json's `grammars`, pointing at
// `../ciac.tmLanguage.json`) and needs no code here.

const { workspace } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const serverPath = workspace.getConfiguration("ciac").get("serverPath", "ciac");

  const serverOptions = {
    command: serverPath,
    args: ["lsp"],
    transport: TransportKind.stdio,
  };

  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "ciac" }],
  };

  client = new LanguageClient("ciac", "CIaC Language Server", serverOptions, clientOptions);
  context.subscriptions.push(client.start());
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
