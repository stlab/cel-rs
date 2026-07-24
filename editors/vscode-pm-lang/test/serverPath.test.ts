import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveServerPath, ResolveServerPathOptions } from '../src/serverPath';

function options(overrides: Partial<ResolveServerPathOptions> = {}): ResolveServerPathOptions {
  return {
    configuredPath: undefined,
    workspaceRoot: undefined,
    platform: 'linux',
    pathEnv: undefined,
    fileExists: () => false,
    ...overrides,
  };
}

test('returns the configured path when it exists', () => {
  const result = resolveServerPath(
    options({
      configuredPath: '/custom/pm-lsp',
      fileExists: (p) => p === '/custom/pm-lsp',
    }),
  );
  assert.equal(result, '/custom/pm-lsp');
});

test('trims whitespace around the configured path before checking it', () => {
  const result = resolveServerPath(
    options({
      configuredPath: ' /custom/pm-lsp ',
      fileExists: (p) => p === '/custom/pm-lsp',
    }),
  );
  assert.equal(result, '/custom/pm-lsp');
});

test('returns undefined when the configured path does not exist, without falling back', () => {
  const result = resolveServerPath(
    options({
      configuredPath: '/custom/pm-lsp',
      workspaceRoot: '/repo',
      fileExists: (p) => p === '/repo/target/debug/pm-lsp',
    }),
  );
  assert.equal(result, undefined);
});

test('falls back to the workspace debug target when no path is configured', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: '/repo',
      fileExists: (p) => p === '/repo/target/debug/pm-lsp',
    }),
  );
  assert.equal(result, '/repo/target/debug/pm-lsp');
});

test('falls back to the workspace release target when debug is missing', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: '/repo',
      fileExists: (p) => p === '/repo/target/release/pm-lsp',
    }),
  );
  assert.equal(result, '/repo/target/release/pm-lsp');
});

test('appends .exe on win32', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: 'C:\\repo',
      platform: 'win32',
      fileExists: (p) => p === 'C:\\repo\\target\\debug\\pm-lsp.exe',
    }),
  );
  assert.equal(result, 'C:\\repo\\target\\debug\\pm-lsp.exe');
});

test('falls back to searching PATH when no workspace match exists', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: '/repo',
      pathEnv: '/usr/local/bin:/usr/bin',
      fileExists: (p) => p === '/usr/bin/pm-lsp',
    }),
  );
  assert.equal(result, '/usr/bin/pm-lsp');
});

test('trims whitespace around PATH entries before joining', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: '/repo',
      pathEnv: ' /usr/local/bin : /usr/bin ',
      fileExists: (p) => p === '/usr/bin/pm-lsp',
    }),
  );
  assert.equal(result, '/usr/bin/pm-lsp');
});

test('returns undefined when nothing is found anywhere', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: '/repo',
      pathEnv: '/usr/bin',
    }),
  );
  assert.equal(result, undefined);
});
