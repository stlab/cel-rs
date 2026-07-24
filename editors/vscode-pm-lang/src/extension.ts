import * as fs from 'node:fs';
import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';
import { resolveServerPath } from './serverPath';

let client: LanguageClient | undefined;

/** Activates the pm-lang extension: resolves the `pm-lsp` binary and starts the language client. */
export function activate(context: vscode.ExtensionContext): void {
  const configuredPath = vscode.workspace.getConfiguration('pm-lang').get<string>('serverPath');
  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

  const serverPath = resolveServerPath({
    configuredPath: configuredPath || undefined,
    workspaceRoot,
    platform: process.platform,
    pathEnv: process.env.PATH,
    fileExists: (candidate) => fs.existsSync(candidate),
  });

  if (!serverPath) {
    if (configuredPath) {
      vscode.window.showErrorMessage(
        `pm-lang: the configured "pm-lang.serverPath" (${configuredPath}) does not exist.`,
      );
    } else {
      vscode.window.showErrorMessage(
        'pm-lang: could not find the pm-lsp language server binary. Build it with ' +
          '"cargo build -p pm-lsp", or set the "pm-lang.serverPath" setting.',
      );
    }
    return;
  }

  const serverOptions: ServerOptions = { command: serverPath };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'pm-lang' }],
  };

  client = new LanguageClient('pm-lang', 'pm-lang Language Server', serverOptions, clientOptions);
  context.subscriptions.push({ dispose: () => void client?.stop() });
  client.start().catch((error: unknown) => {
    vscode.window.showErrorMessage(
      `pm-lang: failed to start the pm-lsp language server: ${error instanceof Error ? error.message : String(error)}`,
    );
  });
}

/** Deactivates the extension, stopping the language client if it's running. */
export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
