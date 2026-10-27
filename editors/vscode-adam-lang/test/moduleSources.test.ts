import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadModuleSources, createModuleSourceRefresh, createModuleSourceHandlers, errorMessage, isConfiguredModuleFile } from '../src/moduleSources';
import { mkdir, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import * as path from 'node:path';

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
