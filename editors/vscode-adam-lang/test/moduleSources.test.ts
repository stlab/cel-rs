import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadModuleSources, ModuleSourceLoader, createModuleSourceRefresh, createModuleSourceHandlers, errorMessage, isConfiguredModuleFile } from '../src/moduleSources';
import { mkdir, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import * as path from 'node:path';
import type { ModuleSourceOptions } from '../src/moduleSources';
import { INITIAL, parseRawGrammar, Registry } from 'vscode-textmate';
import { loadWASM, OnigScanner, OnigString } from 'vscode-oniguruma';

/** Reproduces the extension's shared loader and event wiring without VS Code. */
function moduleAdapter(currentOptions: () => ModuleSourceOptions, requestRefresh: () => void) {
  const sources = new ModuleSourceLoader(currentOptions);
  return {
    ...createModuleSourceHandlers(sources, requestRefresh),
    load: () => sources.load(),
  };
}

/** Supplies explicit workspace-relative module configuration and owned source text. */
function options(modules: unknown = { constraints: 'constraints.adm2' }) {
  return {
    modules,
    workspaceRoot: '/workspace',
    toUri: (file: string) => pathToFileURL(file).href,
    realPath: async (file: string) => file,
    readText: async () => 'sheet ordered(a: f64, b: f64) {}',
  };
}

test('configured paths produce the exact caller-owned initialization wire records', async () => {
  const definitionUri = 'file:///workspace/constraints.adm2';
  const records = await loadModuleSources(options());
  assert.deepEqual(records, [{
    path: 'constraints',
    uri: definitionUri,
    text: 'sheet ordered(a: f64, b: f64) {}',
  }]);
});

test('physical workspace containment rejects file and directory escapes and recovers visibly', async () => {
  const fixture = path.resolve(`.module-source-fixture-${randomUUID()}`);
  const workspace = path.join(fixture, 'workspace');
  const outside = path.join(fixture, 'outside');
  await mkdir(workspace, { recursive: true });
  try {
    await mkdir(outside);
    await writeFile(path.join(outside, 'private.adm2'), 'NONSECRET_EXTERNAL_SENTINEL');
    await writeFile(path.join(workspace, 'inside.adm2'), 'sheet inside() {}');
    await symlink(path.join(outside, 'private.adm2'), path.join(workspace, 'escape.adm2'), 'file');
    await symlink(outside, path.join(workspace, 'escape-dir'), 'dir');
    await symlink(path.join(workspace, 'inside.adm2'), path.join(workspace, 'internal.adm2'), 'file');
    await mkdir(path.join(workspace, 'directory'));
    await writeFile(path.join(workspace, 'directory', 'nested.adm2'), 'sheet nested() {}');
    await symlink(path.join(workspace, 'directory'), path.join(workspace, 'internal-dir'), 'dir');
    await symlink(workspace, path.join(fixture, 'linked-root'), 'dir');
    const root = path.join(fixture, 'linked-root');
    let file = 'escape.adm2';
    const snapshots: unknown[] = [];
    const errors: string[] = [];
    const configuration = () => ({
      modules: { lib: file },
      workspaceRoot: root,
      toUri: (value: string) => pathToFileURL(value).href,
      realPath: realpath,
      readText: async (value: string) => readFile(value, 'utf8'),
    });

    const load = () => loadModuleSources(configuration());
    const refresh = createModuleSourceRefresh(load, async (value) => { snapshots.push(value); }, (value) => { errors.push(value); });
    for (const escape of ['escape.adm2', 'escape-dir/private.adm2']) {
      file = escape;
      await assert.rejects(load(), /inside the workspace/);
      await refresh();
      assert.match(errors.at(-1)!, /inside the workspace/);
      assert.ok('moduleSourcesError' in (snapshots.at(-1) as object));
      assert.ok(!JSON.stringify(snapshots).includes('NONSECRET_EXTERNAL_SENTINEL'));
    }
    file = 'internal.adm2';
    await refresh();
    assert.deepEqual(snapshots.at(-1), { moduleSources: [{
      path: 'lib', uri: pathToFileURL(path.join(root, file)).href, text: 'sheet inside() {}',
    }] });
    file = 'internal-dir/nested.adm2';
    await refresh();
    assert.deepEqual(snapshots.at(-1), { moduleSources: [{
      path: 'lib', uri: pathToFileURL(path.join(root, file)).href, text: 'sheet nested() {}',
    }] });
    file = 'missing.adm2';
    await assert.rejects(load(), /Cannot read module/);
    await refresh();
    assert.ok('moduleSourcesError' in (snapshots.at(-1) as object));
    await writeFile(path.join(workspace, file), 'sheet recovered() {}');
    await refresh();
    assert.deepEqual(snapshots.at(-1), { moduleSources: [{
      path: 'lib', uri: pathToFileURL(path.join(root, file)).href, text: 'sheet recovered() {}',
    }] });
    await rm(root);
    await refresh();
    assert.ok('moduleSourcesError' in (snapshots.at(-1) as object));
    await symlink(workspace, root, 'dir');
    await refresh();
    assert.deepEqual(snapshots.at(-1), { moduleSources: [{
      path: 'lib', uri: pathToFileURL(path.join(root, file)).href, text: 'sheet recovered() {}',
    }] });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('loader reads the approved canonical target while keeping the lexical overlay URI', async () => {
  const records = await loadModuleSources({
    ...options({ lib: 'linked.adm2' }),
    realPath: async (file) => file === '/workspace' ? '/physical' : '/physical/approved.adm2',
    readText: async (file) => {
      assert.equal(file, '/physical/approved.adm2');
      return 'sheet approved() {}';
    },
  });
  assert.equal(records[0].uri, 'file:///workspace/linked.adm2');
  assert.equal(records[0].text, 'sheet approved() {}');
});

test('a suspended load keeps its original workspace containment and owner snapshot', async () => {
  let releaseRoot!: (root: string) => void;
  const root = new Promise<string>((resolve) => { releaseRoot = resolve; });
  const reads: string[] = [];
  let configuration: ModuleSourceOptions = {
    ...options({ original: 'original.adm2' }), workspaceRoot: '/repo',
    realPath: async (file) => {
      if (file === '/repo') { return root; }
      if (file === '/repo/sub') { return file; }
      return file === '/repo/original.adm2' ? '/repo/original.txt' : '/repo/outside-new-root.txt';
    },
    readText: async (file) => { reads.push(file); return 'sheet original() {}'; },
  };
  const sources = new ModuleSourceLoader(() => configuration);
  const pending = sources.load();
  configuration = { ...configuration, workspaceRoot: '/repo/sub', modules: { replacement: 'linked.adm2' } };
  assert.ok(sources.isSource('file:///repo/sub/linked.adm2'));
  releaseRoot('/repo');
  const records = await pending;
  assert.deepEqual(reads, ['/repo/original.txt'], 'a new owner must never use the previous workspace boundary');
  assert.equal(records[0].uri, 'file:///repo/original.adm2');
  assert.ok(!sources.isSource('file:///repo/original.txt'), 'detached owners must not publish current aliases');
  await assert.rejects(sources.load(), /inside the workspace/);
  assert.deepEqual(reads, ['/repo/original.txt'], 'the new workspace escape must never be read');
});

test('failure of a suspended old root does not mark replacement owners unresolved', async () => {
  let rejectRoot!: (error: Error) => void;
  const root = new Promise<string>((_resolve, reject) => { rejectRoot = reject; });
  let configuration: ModuleSourceOptions = {
    ...options({ original: 'original.adm2' }), workspaceRoot: '/repo',
    realPath: async (file) => file === '/repo' ? root : file,
  };
  const sources = new ModuleSourceLoader(() => configuration);
  const pending = assert.rejects(sources.load(), /old root disappeared/);
  configuration = { ...configuration, workspaceRoot: '/repo/sub', modules: { replacement: 'new.adm2' } };
  assert.ok(sources.isSource('file:///repo/sub/new.adm2'));
  rejectRoot(new Error('old root disappeared'));
  await pending;
  assert.ok(!sources.isSource('file:///repo/unrelated.txt'), 'an old failure must not mutate a new owner generation');
  await sources.load();
  assert.ok(!sources.isSource('file:///repo/unrelated.txt'));
});

test('workspace-root watcher spellings retain canonical ownership through failures and removal', async () => {
  let target = '/physical/sources/library.txt';
  let failure: 'read' | 'resolve' | undefined;
  let modules: unknown = { lib: 'linked/library.txt' };
  const sources = new ModuleSourceLoader(() => ({
    ...options(modules),
    realPath: async (file) => {
      if (file === '/workspace') { return '/physical'; }
      if (failure === 'resolve') { throw new Error('deleted target'); }
      return target;
    },
    readText: async () => {
      if (failure === 'read') { throw new Error('permission denied'); }
      return 'sheet library() {}';
    },
  }));
  const records = await sources.load();
  assert.equal(records[0].uri, 'file:///workspace/linked/library.txt');
  assert.ok(sources.isSource('file:///workspace/sources/library.txt'), 'watchers preserve the lexical workspace root');
  assert.ok(sources.isSource('file:///workspace/sources'), 'rebased ancestor replacements must refresh');
  failure = 'read';
  await assert.rejects(sources.load(), /permission denied/);
  assert.ok(sources.isSource('file:///workspace/sources/library.txt'));
  assert.ok(!sources.isSource('file:///workspace/unrelated.txt'));
  failure = 'resolve';
  await assert.rejects(sources.load(), /deleted target/);
  assert.ok(sources.isSource('file:///workspace/sources/library.txt'));
  failure = undefined;
  await sources.load();
  assert.ok(sources.isSource('file:///workspace/sources/library.txt'));
  target = '/physical/replacement.txt';
  await sources.load();
  assert.ok(!sources.isSource('file:///workspace/sources/library.txt'));
  assert.ok(sources.isSource('file:///workspace/replacement.txt'));
  modules = {};
  assert.ok(!sources.isSource('file:///workspace/replacement.txt'));
});

test('canonical events refresh lexical owners through directory and workspace links and prune removed owners', async () => {
  const fixture = path.resolve(`.module-source-fixture-${randomUUID()}`);
  const physical = path.join(fixture, 'physical');
  const root = path.join(fixture, 'workspace');
  await mkdir(path.join(physical, 'sources'), { recursive: true });
  try {
    const target = path.join(physical, 'sources', 'library.txt');
    await writeFile(target, 'sheet original() {}');
    await symlink(physical, root, 'dir');
    await symlink(path.join(physical, 'sources'), path.join(physical, 'linked'), 'dir');
    await symlink(target, path.join(physical, 'alias.adm2'), 'file');
    let modules: unknown = { a: 'linked/library.txt', b: 'alias.adm2' };
    let refreshes = 0;
    const adapter = moduleAdapter(() => ({
      ...options(modules), workspaceRoot: root, realPath: realpath,
      readText: (file) => readFile(file, 'utf8'),
    }), () => { refreshes += 1; });
    const records = await adapter.load();
    assert.deepEqual(records.map((record) => record.uri), [
      pathToFileURL(path.join(root, 'linked/library.txt')).href,
      pathToFileURL(path.join(root, 'alias.adm2')).href,
    ]);
    adapter.sourceChanged(pathToFileURL(path.join(root, 'sources/library.txt')).href);
    assert.equal(refreshes, 1, 'a watcher target edit/save beneath the lexical workspace root must refresh its owners');
    adapter.sourceChanged(pathToFileURL(path.join(root, 'linked')).href);
    assert.equal(refreshes, 2, 'replacing a directory symlink must refresh its descendants');
    adapter.sourceChanged(pathToFileURL(path.join(physical, 'unrelated.adm2')).href);
    assert.equal(refreshes, 2);
    modules = { b: 'alias.adm2' };
    adapter.configurationChanged(true);
    await adapter.load();
    adapter.sourceChanged(pathToFileURL(target).href);
    assert.equal(refreshes, 4, 'removing one owner must not discard another owner');
    modules = {};
    adapter.configurationChanged(true);
    adapter.sourceChanged(pathToFileURL(target).href);
    assert.equal(refreshes, 5, 'configuration changes prune obsolete targets before loading');
    await adapter.load();
    assert.equal(refreshes, 5);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('read failures and deleted targets retain event ownership until recovery and retargeting', async () => {
  let target = '/physical/original.txt';
  let failure: 'read' | 'resolve' | undefined = 'read';
  let refreshes = 0;
  const adapter = moduleAdapter(() => ({
    ...options({ lib: 'linked.adm2', second: 'second.adm2' }),
    realPath: async (file) => {
      if (file === '/workspace') { return '/physical'; }
      if (failure === 'resolve') { throw new Error('deleted target'); }
      return file.endsWith('second.adm2') ? '/physical/second.txt' : target;
    },
    readText: async () => {
      if (failure === 'read') { throw new Error('permission denied'); }
      return 'sheet recovered() {}';
    },
  }), () => { refreshes += 1; });
  const snapshots: unknown[] = [];
  const errors: string[] = [];
  const refresh = createModuleSourceRefresh(adapter.load, async (value) => { snapshots.push(value); }, (value) => { errors.push(value); });
  await refresh();
  assert.match(errors[0], /permission denied/);
  adapter.sourceChanged('file:///physical/original.txt');
  adapter.sourceChanged('file:///physical/second.txt');
  assert.equal(refreshes, 2, 'a failed first read must not strand either owner');
  failure = 'resolve';
  await refresh();
  assert.ok('moduleSourcesError' in (snapshots.at(-1) as object));
  adapter.sourceChanged('file:///physical/original.txt');
  assert.equal(refreshes, 3, 'deletion must keep the last approved canonical owner');
  failure = undefined;
  adapter.sourceChanged('file:///physical/original.txt');
  assert.equal(refreshes, 4, 'recreation must request recovery');
  await refresh();
  assert.ok('moduleSources' in (snapshots.at(-1) as object));
  target = '/physical/replacement.txt';
  adapter.sourceChanged('file:///workspace/linked.adm2');
  await refresh();
  adapter.sourceChanged('file:///physical/original.txt');
  assert.equal(refreshes, 5, 'successful retargeting removes the old canonical alias');
  adapter.sourceChanged('file:///physical/replacement.txt');
  assert.equal(refreshes, 6);
  adapter.sourceChanged('file:///physical/unrelated.adm2');
  assert.equal(refreshes, 6);
});

test('initial resolution failures retry unknown target events until resolution restores precise filtering', async () => {
  for (const failedPath of ['/workspace', '/workspace/linked.adm2']) {
    let broken = true;
    let refreshes = 0;
    const adapter = moduleAdapter(() => ({
      ...options({ lib: 'linked.adm2' }),
      realPath: async (file) => {
        if (broken && file === failedPath) { throw new Error('missing workspace or target'); }
        return file === '/workspace' ? '/physical' : '/physical/recovered.txt';
      },
    }), () => { refreshes += 1; });
    await assert.rejects(adapter.load(), /missing workspace or target/);
    adapter.sourceChanged('file:///physical/recovered.txt');
    assert.equal(refreshes, 1, 'unknown initial realpaths must not suppress recovery events');
    broken = false;
    await adapter.load();
    adapter.sourceChanged('file:///physical/unrelated.adm2');
    assert.equal(refreshes, 1);
    adapter.sourceChanged('file:///physical/recovered.txt');
    assert.equal(refreshes, 2);
  }
});

test('configuration and read failures reject rather than manufacture empty successful sources', async () => {
  for (const modules of [null, [], 'x', { '': 'a' }, { _: 'a' }, { 'bad-path': 'a' }, { lib: '' }, { lib: '../outside' }, { lib: '/absolute' }, { lib: 1 }, { a: 'same', b: 'same' }]) {
    await assert.rejects(loadModuleSources(options(modules)));
  }
  await assert.rejects(loadModuleSources({ ...options(), workspaceRoot: undefined }));
  await assert.rejects(loadModuleSources({ ...options(), readText: async () => { throw new Error('permission denied'); } }), /constraints.*permission denied/);
  assert.deepEqual(await loadModuleSources(options({})), []);
});

test('Unicode module paths use the same explicit owner protocol', async () => {
  const records = await loadModuleSources(options({ 'géométrie::位置': 'unicode.adm2' }));
  assert.equal(records[0].path, 'géométrie::位置');
});

test('file events select only explicitly mapped files, including deleted sources and non-adm2 names', () => {
  const configured = options({ lib: 'library.txt' });
  assert.ok(isConfiguredModuleFile('file:///workspace/library.txt', configured));
  assert.ok(!isConfiguredModuleFile('file:///workspace/other.adm2', configured));
  assert.ok(!isConfiguredModuleFile('file:///workspace/other.adm2', options(null)));
  assert.ok(!isConfiguredModuleFile('file:///workspace/library.txt', { ...configured, workspaceRoot: undefined }));
});

test('editor event glue uses current configuration and suppresses unrelated source/configuration changes', () => {
  let configuration = options({ lib: 'old.adm2' });
  let refreshes = 0;
  const handlers = createModuleSourceHandlers(new ModuleSourceLoader(() => configuration), () => { refreshes += 1; });
  handlers.sourceChanged('file:///workspace/unmapped.adm2');
  handlers.configurationChanged(false);
  assert.equal(refreshes, 0);
  handlers.sourceChanged('file:///workspace/old.adm2');
  assert.equal(refreshes, 1);
  configuration = options({ lib: 'new.adm2' });
  handlers.configurationChanged(true);
  handlers.sourceChanged('file:///workspace/old.adm2');
  handlers.sourceChanged('file:///workspace/new.adm2');
  assert.equal(refreshes, 3);
});

test('editor event handlers accept an owner providing only source identity', () => {
  let selected = 'file:///workspace/first.adm2';
  let refreshes = 0;
  const handlers = createModuleSourceHandlers(
    { isSource: (uri: string) => uri === selected },
    () => { refreshes += 1; },
  );
  handlers.sourceChanged(selected);
  selected = 'file:///workspace/second.adm2';
  handlers.sourceChanged('file:///workspace/first.adm2');
  handlers.sourceChanged(selected);
  assert.equal(refreshes, 2);
});

test('refresh transport failures reject but do not poison future refreshes', async () => {
  let count = 0;
  const refresh = createModuleSourceRefresh(
    async () => [],
    async () => { count += 1; if (count === 1) { throw new Error('transport'); } },
    () => { assert.fail('transport failures are not source loading failures'); },
  );
  await assert.rejects(refresh(), /transport/);
  await refresh();
  assert.equal(count, 2);
  assert.equal(errorMessage('plain failure'), 'plain failure');
});

test('disposing refresh ownership suppresses pending publications and future loads', async () => {
  for (const rejected of [false, true]) {
    let release: (() => void) | undefined;
    let started: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const loading = new Promise<void>((resolve) => { started = resolve; });
    let loads = 0;
    const refresh = createModuleSourceRefresh(
      async () => {
        loads += 1;
        started?.();
        await gate;
        if (rejected) { throw new Error('disposed load failed'); }
        return [];
      },
      async () => { assert.fail('disposed refresh must not publish'); },
      () => { assert.fail('disposed refresh must not report'); },
    );
    const pending = [refresh(), refresh()];
    await loading;
    refresh.dispose();
    release?.();
    await Promise.all(pending);
    await refresh();
    assert.equal(loads, 1);
  }
});

test('refreshes serialize snapshots and publish read errors explicitly before recovery', async () => {
  const first = { path: 'first', uri: 'file:///first.adm2', text: 'sheet first() {}' };
  const last = { path: 'last', uri: 'file:///last.adm2', text: 'sheet last() {}' };
  const snapshots: unknown[] = [];
  const errors: string[] = [];
  let release: (() => void) | undefined;
  let started: (() => void) | undefined;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const publishing = new Promise<void>((resolve) => { started = resolve; });
  let count = 0;
  const refresh = createModuleSourceRefresh(
    async () => {
      count += 1;
      if (count === 2) { throw new Error('deleted definition'); }
      return count === 1 ? [first] : [last];
    },
    async (snapshot) => {
      snapshots.push(snapshot);
      if (snapshots.length === 1) {
        started?.();
        await gate;
      }
    },
    (message) => { errors.push(message); },
  );
  const pending = [refresh(), refresh(), refresh()];
  await publishing;
  try {
    assert.equal(count, 1, 'a pending publication owns the queue');
    assert.deepEqual(snapshots, [{ moduleSources: [first] }]);
  } finally {
    release?.();
    await Promise.all(pending);
  }
  assert.deepEqual(snapshots, [
    { moduleSources: [first] },
    { moduleSourcesError: 'deleted definition' },
    { moduleSources: [last] },
  ]);
  assert.deepEqual(errors, ['deleted definition']);
});

test('grammar highlights module keywords, parameter types and named bindings', async () => {
  await loadWASM(await readFile(require.resolve('vscode-oniguruma/release/onig.wasm')));
  const grammarPath = path.resolve(__dirname, '../../syntaxes/adam-lang.tmLanguage.json');
  const source = await readFile(grammarPath, 'utf8');
  const registry = new Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (patterns) => new OnigScanner(patterns),
      createOnigString: (content) => new OnigString(content),
    }),
    loadGrammar: async (scope) => scope === 'source.adam-lang'
      ? parseRawGrammar(source, grammarPath) : null,
  });
  try {
    const grammar = await registry.loadGrammar('source.adam-lang');
    assert.ok(grammar);
    /** Returns observable token text and scopes for one independently tokenized line. */
    const tokens = (line: string) => grammar.tokenizeLine(line, INITIAL).tokens
      .map((token) => ({ text: line.slice(token.startIndex, token.endIndex), scopes: token.scopes }))
      .filter((token) => token.text.length > 0);
    const imports = tokens('use geometry as g;');
    for (const keyword of ['use', 'as']) {
      assert.ok(imports.some((token) => token.text === keyword
        && token.scopes.includes('keyword.declaration.adam-lang')));
    }
    const parameters = tokens('sheet area(a: f64) {}');
    assert.ok(parameters.some((token) => token.text === 'f64'
      && token.scopes.includes('storage.type.adam-lang')));
    assert.ok(parameters.some((token) => token.text === 'a'
      && token.scopes.includes('variable.parameter.adam-lang')));
    for (const [line, scope] of [
      ['// use geometry as f64; a: x', 'comment.line.double-slash.adam-lang'],
      ['"use geometry as f64; a: x"', 'string.quoted.double.adam-lang'],
    ]) {
      const protectedTokens = tokens(line);
      assert.ok(protectedTokens.length > 0);
      assert.ok(protectedTokens.every((token) => token.scopes.includes(scope)));
    }
  } finally {
    registry.dispose();
  }
});
