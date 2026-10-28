import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { runInNewContext } from 'node:vm';
import { pathToFileURL } from 'node:url';

/** Supplies an independently owned extension host with a controllable client startup. */
interface ExtensionOptions {
  readonly configuredPath?: string;
  readonly workspaceRoot?: string;
  readonly modules?: unknown;
  readonly readSource?: (file: string) => Promise<Uint8Array>;
  readonly start?: () => Promise<void>;
  readonly stop?: () => Promise<void>;
}

/** Creates a caller-owned, one-shot gate for an explicitly controlled suspension. */
function createGate() {
  let open: (() => void) | undefined;
  const promise = new Promise<void>((resolve) => { open = resolve; });
  assert.ok(open);
  return { promise, open };
}

/**
 * Creates an isolated instance of the real compiled extension with host-only adapters.
 * Filesystem/path resolution and module snapshot loading use the production modules.
 * The caller owns subscriptions and must deactivate/dispose the instance.
 * Complexity: O(n) time and space in compiled extension source bytes.
 */
function createExtension(options: ExtensionOptions = {}) {
  const errors: string[] = [];
  const subscriptions: { dispose(): void }[] = [];
  const notifications: { method: string; settings: unknown }[] = [];
  const state: {
    starts: number;
    stops: number;
    watchers: number;
    command?: string;
    initial?: unknown;
  } = { starts: 0, stops: 0, watchers: 0 };

  /** Creates a disposable representing one caller-owned editor registration. */
  const register = () => ({ dispose() {} });
  const host = {
    workspace: {
      workspaceFolders: options.workspaceRoot === undefined ? undefined :
        [{ uri: { fsPath: options.workspaceRoot } }],
      /** Returns only explicitly configured settings, preserving the caller's fallback. */
      getConfiguration: () => ({
        get: (name: string, fallback?: unknown) =>
          name === 'serverPath' ? options.configuredPath ?? process.execPath :
            name === 'modules' ? options.modules ?? fallback : fallback,
      }),
      fs: {
        /** Loads the selected source through the case's controlled editor IO boundary. */
        readFile: (uri: { fsPath: string }) => (options.readSource ?? readFile)(uri.fsPath),
      },
      /** Creates the watcher owned by the extension context. */
      createFileSystemWatcher: () => {
        state.watchers++;
        return {
          dispose() {},
          onDidCreate: register,
          onDidChange: register,
          onDidDelete: register,
        };
      },
      onDidChangeConfiguration: register,
      onDidChangeWorkspaceFolders: register,
      onDidSaveTextDocument: register,
    },
    window: {
      /** Retains visible startup/transport diagnostics for the owning case. */
      showErrorMessage: async (message: string) => { errors.push(message); },
    },
    Uri: {
      /** Preserves the mapped file's lexical filesystem and URI identity. */
      file: (file: string) => ({ fsPath: file, toString: () => pathToFileURL(file).href }),
    },
  };

  /** Creates the client boundary without launching an external language-server process. */
  function createClient(
    _id: string,
    _name: string,
    serverOptions: { command: string },
    clientOptions: { initializationOptions: unknown },
  ) {
    state.command = serverOptions.command;
    state.initial = clientOptions.initializationOptions;
    let starting = false;
    return {
      /** Starts according to the test's explicit external-process outcome. */
      start: async () => {
        state.starts++;
        starting = true;
        try {
          await options.start?.();
        } finally {
          starting = false;
        }
      },
      /** Releases process ownership, rejecting startup overlap as LanguageClient does. */
      stop: async () => {
        if (starting) throw new Error('Cannot stop a client during startup');
        state.stops++;
        await options.stop?.();
      },
      /** Retains each production snapshot publication without deriving its expected value. */
      sendNotification: async (method: string, parameters: { settings: unknown }) => {
        notifications.push({ method, settings: parameters.settings });
      },
    };
  }

  const extensionPath = require.resolve('../src/extension');
  const productionRequire = createRequire(extensionPath);
  const exports: Partial<typeof import('../src/extension')> = {};
  runInNewContext(readFileSync(extensionPath, 'utf8'), {
    exports,
    /** Substitutes only APIs supplied by the editor and its external process adapter. */
    require: (name: string) => {
      if (name === 'vscode') return host;
      if (name === 'vscode-languageclient/node') return { LanguageClient: createClient };
      return productionRequire(name);
    },
    process: { platform: process.platform, env: { PATH: '' } },
    TextDecoder,
    AbortController,
  }, { filename: extensionPath });
  assert.ok(exports.activate);
  assert.ok(exports.deactivate);
  return {
    activate: exports.activate,
    deactivate: exports.deactivate,
    context: { subscriptions },
    errors,
    notifications,
    state,
  };
}

/**
 * Releases the current client and all caller-owned context registrations, even after shutdown failure.
 * Complexity: O(n) time and space in registrations.
 */
async function closeExtension(extension: ReturnType<typeof createExtension>) {
  try {
    await extension.deactivate();
  } finally {
    extension.context.subscriptions.splice(0).forEach((subscription) => subscription.dispose());
  }
}

/** Checks startup/refresh wire data and release using only subscription ownership. */
test('activation publishes its current snapshot and deactivation releases the client', async () => {
  const extension = createExtension();
  try {
    await extension.activate(extension.context);
    assert.equal(extension.state.command, process.execPath);
    assert.deepEqual(extension.state.initial, { moduleSources: [] });
    assert.deepEqual(extension.notifications, [{
      method: 'workspace/didChangeConfiguration',
      settings: { moduleSources: [] },
    }]);
    assert.equal(extension.state.starts, 1);
    assert.equal(extension.state.watchers, 1);
    assert.deepEqual(extension.errors, []);
    await extension.deactivate();
    assert.equal(extension.state.stops, 1);
  } finally {
    await closeExtension(extension);
  }
});

/** Checks automatic discovery failure tells the user how to supply the missing executable. */
test('activation reports an unavailable automatic server without starting a client', async () => {
  const extension = createExtension({ configuredPath: '' });
  await extension.activate(extension.context);
  assert.equal(extension.state.starts, 0);
  assert.equal(extension.state.watchers, 0);
  assert.equal(extension.errors.length, 1);
  assert.match(extension.errors[0], /could not find.*adam-lsp/);
  assert.ok(extension.errors[0].includes('cargo build -p adam-lsp'));
  assert.equal(extension.deactivate(), undefined);
});

/** Checks an unavailable configured executable remains an error rather than a fallback launch. */
test('activation reports a missing configured server without starting a client', async () => {
  const missing = join(tmpdir(), randomUUID(), 'adam-lsp');
  const extension = createExtension({ configuredPath: missing });
  await extension.activate(extension.context);
  assert.equal(extension.state.starts, 0);
  assert.equal(extension.state.watchers, 0);
  assert.equal(extension.errors.length, 1);
  assert.match(extension.errors[0], /configured.*serverPath/);
  assert.ok(extension.errors[0].includes(missing));
  assert.equal(extension.deactivate(), undefined);
});

/** Checks failed startup reports its cause, releases the client and installs no watcher. */
test('activation reports and closes a client whose startup fails', async () => {
  const extension = createExtension({
    start: async () => { throw new Error('test server startup failed'); },
  });
  try {
    await extension.activate(extension.context);
    assert.equal(extension.state.starts, 1);
    assert.equal(extension.state.stops, 1);
    assert.equal(extension.state.watchers, 0);
    assert.deepEqual(extension.notifications, []);
    assert.equal(extension.errors.length, 1);
    assert.match(extension.errors[0], /test server startup failed/);
  } finally {
    await closeExtension(extension);
  }
});

/** Checks a shutdown failure propagates through the public lifetime boundary. */
test('deactivation propagates a language client shutdown failure', async () => {
  const extension = createExtension({
    stop: async () => { throw new Error('test server shutdown failed'); },
  });
  try {
    await extension.activate(extension.context);
    await assert.rejects(async () => extension.deactivate(), /test server shutdown failed/);
  } finally {
    await closeExtension(extension);
  }
});

/** Checks a released initial load cannot start a server after its owner has closed. */
test('deactivation during initial source IO prevents subsequent startup', { timeout: 5_000 }, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'adam-extension-'));
  const source = join(directory, 'example.adm2');
  const entered = createGate();
  const complete = createGate();
  let extension: ReturnType<typeof createExtension> | undefined;
  let activation: Promise<void> | undefined;
  try {
    await writeFile(source, 'sheet example {}');
    let sourceRead = false;
    extension = createExtension({
      workspaceRoot: directory,
      modules: { example: 'example.adm2' },
      readSource: async (file) => {
        sourceRead = true;
        entered.open();
        await complete.promise;
        return readFile(file);
      },
    });
    activation = extension.activate(extension.context);
    await Promise.race([entered.promise, activation]);
    assert.ok(sourceRead, 'The fixture must reach the initial source IO boundary');
    const shutdown = extension.deactivate();
    complete.open();
    await Promise.all([activation, shutdown]);
    assert.equal(extension.state.starts, 0);
    assert.equal(extension.state.watchers, 0);
    assert.deepEqual(extension.notifications, []);
    assert.deepEqual(extension.errors, []);
  } finally {
    complete.open();
    try {
      await activation;
    } finally {
      try {
        if (extension) await closeExtension(extension);
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    }
  }
});

/** Checks a finishing startup cannot register listeners or publish after its owner closes. */
test('deactivation during startup suppresses event registration and refresh', { timeout: 5_000 }, async () => {
  const entered = createGate();
  const complete = createGate();
  const extension = createExtension({
    start: async () => {
      entered.open();
      await complete.promise;
    },
  });
  const activation = extension.activate(extension.context);
  try {
    await Promise.race([entered.promise, activation]);
    assert.equal(extension.state.starts, 1, 'The fixture must reach client startup');
    const shutdown = extension.deactivate();
    complete.open();
    await Promise.all([activation, shutdown]);
    assert.equal(extension.state.watchers, 0);
    assert.deepEqual(extension.notifications, []);
    assert.deepEqual(extension.errors, []);
  } finally {
    complete.open();
    try {
      await activation;
    } finally {
      await closeExtension(extension);
    }
  }
});
