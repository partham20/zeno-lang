// The Zeno extension for VS Code. All it does is start `zeno lsp` and
// let VS Code talk to it: the mistakes, and the layout on save, come
// from the language server. The colours come from the grammar in
// syntaxes/, which needs no code.
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate() {
    const zeno = vscode.workspace.getConfiguration("zeno").get("path") || "zeno";
    client = new LanguageClient(
        "zeno",
        "Zeno",
        { command: zeno, args: ["lsp"] },
        { documentSelector: [{ scheme: "file", language: "zeno" }] }
    );
    client.start();
}

function deactivate() {
    return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
