import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadModuleSources, createModuleSourceRefresh, createModuleSourceHandlers, errorMessage, isConfiguredModuleFile } from '../src/moduleSources';
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import * as path from 'node:path';

/** Supplies explicit workspace-relative module configuration and owned source text. */
function options(modules: unknown = { constraints: 'constraints.adm2' }) {
  return {
    modules,
    workspaceRoot: '/workspace',
    toUri: (file: string) => pathToFileURL(file).href,
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
  const handlers = createModuleSourceHandlers(() => configuration, () => { refreshes += 1; });
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
  let release: (() => void) | undefined;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let loads = 0;
  const refresh = createModuleSourceRefresh(
    async () => { loads += 1; await gate; return []; },
    async () => { assert.fail('disposed refresh must not publish'); },
    () => { assert.fail('disposed refresh must not report'); },
  );
  const pending = refresh();
  await Promise.resolve();
  refresh.dispose();
  release?.();
  await pending;
  await refresh();
  assert.equal(loads, 1);
});

test('refreshes serialize snapshots and publish read errors explicitly before recovery', async () => {
  const snapshots: unknown[] = [];
  const errors: string[] = [];
  let count = 0;
  const refresh = createModuleSourceRefresh(
    async () => {
      count += 1;
      if (count === 2) { throw new Error('deleted definition'); }
      return await loadModuleSources(options());
    },
    async (snapshot) => { snapshots.push(snapshot); },
    (message) => { errors.push(message); },
  );
  await Promise.all([refresh(), refresh(), refresh()]);
  assert.deepEqual(snapshots, [
    { moduleSources: await loadModuleSources(options()) },
    { moduleSourcesError: 'deleted definition' },
    { moduleSources: await loadModuleSources(options()) },
  ]);
  assert.deepEqual(errors, ['deleted definition']);
});

test('grammar highlights module keywords, parameter types and named bindings', async () => {
  const grammar = JSON.parse(await readFile(path.resolve(__dirname, '../../syntaxes/adam-lang.tmLanguage.json'), 'utf8'));
  const keyword = new RegExp(grammar.repository.keywords.patterns[0].match);
  assert.ok(keyword.test('use'));
  assert.ok(keyword.test('as'));
  assert.ok(new RegExp(grammar.repository.types.patterns[0].match).test('f64'));
  const binding = grammar.repository.bindings;
  assert.ok(binding);
  assert.ok(new RegExp(binding.match).test('a: x'));
  assert.ok(grammar.patterns.some((entry: { include: string }) => entry.include === '#bindings'));
});
