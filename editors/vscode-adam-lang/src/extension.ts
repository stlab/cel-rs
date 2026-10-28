import * as fs from 'node:fs';
import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';
import { resolveServerPath } from './serverPath';
import {
  createModuleSourceRefresh,
  createModuleSourceHandlers,
  errorMessage,
  ModuleSourceLoader,
  ModuleSourceRefresh,
  ModuleSourceSnapshot,
} from './moduleSources';

/** Retains the active language client until extension deactivation. */
let client: LanguageClient | undefined;
/** Retains the refresh queue so deactivation suppresses unfinished load effects. */
let moduleRefresh: ModuleSourceRefresh | undefined;

/**
 * Starts the language client with explicit source owners and serialized snapshot refreshes.
 * Configuration and file read failures remain visible; no failed load becomes empty text.
 * Time/space: source loading plus language-client startup and retained configured source text.
 */
export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const configuredPath = vscode.workspace
    .getConfiguration('adam-lang')
    .get<string>('serverPath')
    ?.trim();
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
        `adam-lang: the configured "adam-lang.serverPath" (${configuredPath}) does not exist.`,
      );
    } else {
      vscode.window.showErrorMessage(
        'adam-lang: could not find the adam-lsp language server binary. Build it with ' +
          '"cargo build -p adam-lsp", or set the "adam-lang.serverPath" setting.',
      );
    }
    return;
  }

  const serverOptions: ServerOptions = { command: serverPath };
  /** Captures the current explicit mapping; URI conversion and IO stay in this tooling adapter. */
  const sourceOptions = () => ({
    modules: vscode.workspace.getConfiguration('adam-lang').get<unknown>('modules', {}),
    workspaceRoot: vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
    toUri: (file: string) => vscode.Uri.file(file).toString(),
    realPath: (file: string) => fs.promises.realpath(file),
    readText: async (file: string) => new TextDecoder('utf-8', { fatal: true }).decode(await vscode.workspace.fs.readFile(vscode.Uri.file(file))),
  });
  const sources = new ModuleSourceLoader(sourceOptions);
  /** Loads one current snapshot and retains its canonical event dependencies across failures. */
  const load = () => sources.load();
  /** Reports source/configuration/transport errors in VS Code's visible error channel. */
  const report = (message: string) => { void vscode.window.showErrorMessage(`adam-lang: ${message}`); };
  // Initialization and refresh share exactly the same loader and wire schema.
  let initial: ModuleSourceSnapshot = { moduleSources: [] };
  await createModuleSourceRefresh(load, async (snapshot) => { initial = snapshot; }, report)();
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'adam-lang' }],
    initializationOptions: initial,
  };

  const languageClient = new LanguageClient('adam-lang', 'adam-lang Language Server', serverOptions, clientOptions);
  client = languageClient;
  const refresh = createModuleSourceRefresh(load, async (snapshot) => {
    await languageClient.sendNotification('workspace/didChangeConfiguration', { settings: snapshot });
  }, report);
  moduleRefresh = refresh;
  context.subscriptions.push({ dispose: () => {
    refresh.dispose();
    void languageClient.stop().catch((error: unknown) => report(errorMessage(error)));
  } });
  try {
    await languageClient.start();
    // Close the startup race between the initial disk snapshot and event registration.
    /** Schedules one complete refresh and reports transport failure without an unhandled rejection. */
    const requestRefresh = () => { void refresh().catch((error: unknown) => report(errorMessage(error))); };
    const handlers = createModuleSourceHandlers(sources, requestRefresh);
    /** Selects mapped dependencies or resolution recovery; watching never discovers modules. */
    const sourceChanged = (uri: vscode.Uri) => handlers.sourceChanged(uri.toString());
    const watcher = vscode.workspace.createFileSystemWatcher('**/*');
    context.subscriptions.push(
      watcher,
      watcher.onDidCreate(sourceChanged),
      watcher.onDidChange(sourceChanged),
      watcher.onDidDelete(sourceChanged),
      vscode.workspace.onDidChangeConfiguration((event) =>
        handlers.configurationChanged(event.affectsConfiguration('adam-lang.modules'))),
      vscode.workspace.onDidChangeWorkspaceFolders(requestRefresh),
      vscode.workspace.onDidSaveTextDocument((document) => sourceChanged(document.uri)),
    );
    await refresh();
  } catch (error: unknown) {
    refresh.dispose();
    report(`failed to start or refresh the adam-lsp language server: ${errorMessage(error)}`);
    await languageClient.stop();
  }
}

/** Deactivates the extension, stopping the language client if it's running. */
export function deactivate(): Thenable<void> | undefined {
  moduleRefresh?.dispose();
  return client?.stop();
}
