# pm-lang VS Code Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a VS Code extension (`editors/vscode-pm-lang/`) that gives `.pm` files syntax
highlighting and live `pm-lsp` diagnostics — the design doc's Phase 3, final sub-plan (`pm-lsp`
itself already ships; this is the client that talks to it).

**Architecture:** A standalone TypeScript npm package, outside the Cargo workspace. A static
TextMate grammar (`syntaxes/pm-lang.tmLanguage.json`) drives syntax highlighting. `src/extension.ts`
resolves the `pm-lsp` binary (via `src/serverPath.ts`, a pure, unit-tested function) and hands it to
`vscode-languageclient`, which spawns it over stdio and wires `textDocument/didOpen`/`didChange` →
`publishDiagnostics` — all of that protocol plumbing already lives in `pm-lsp`
(`pm-lsp/src/dispatch.rs`); the extension only needs to launch the right binary and register the
`pm-lang` document selector.

**Tech Stack:** TypeScript 5.7, `vscode-languageclient` 9.x (Node transport, stdio), `tsc` for
compilation, Node's built-in `node:test` runner for the one piece of pure logic that has unit
tests. No bundler — the extension runs directly from `tsc` output, matching this project's
pre-release, no-distribution-story-yet status.

## Global Constraints

- Language id `pm-lang`, file extension `.pm` — must match the `languageId` string `pm-lsp`'s own
  tests already send (`pm-lsp/src/dispatch.rs`'s `TextDocumentItem { language_id: "pm-lang", .. }`).
- `pm-lsp`'s binary name is `pm-lsp` (`pm-lsp/Cargo.toml`'s `[[bin]] name = "pm-lsp"`), built via
  `cargo build -p pm-lsp` into `target/debug/pm-lsp` (`.exe` on Windows) or `target/release/pm-lsp`.
- `pm-lsp` speaks LSP over stdio unconditionally (`Connection::stdio()` in `pm-lsp/src/dispatch.rs`)
  — no special CLI flags to pass when spawning it.
- Exact dependency versions (verified available on npm): `vscode-languageclient@^9.0.1`,
  `typescript@5.7.3`, `@types/node@20.11.0`, `@types/vscode@1.85.0`, `engines.vscode: ^1.85.0`.
- New directory `editors/vscode-pm-lang/` is **not** added to the root `Cargo.toml` workspace
  `members` list — it's plain TypeScript, already excluded by omission.
- Per the design doc's Testing Strategy: **no automated UI/E2E test suite for the extension** —
  only `src/serverPath.ts` (pure logic, no VS Code API) gets automated unit tests; the grammar and
  the language-client wiring are verified manually (checklist in Task 5), matching the design doc's
  explicit decision.
- No bundler, no `.vsix` packaging step — out of scope per the design doc ("no distribution story
  needed yet").

---

## File Structure

```
editors/vscode-pm-lang/
├── package.json                       # extension manifest + npm scripts + deps
├── tsconfig.json
├── language-configuration.json        # brackets, comments, auto-closing pairs
├── README.md
├── .vscode/
│   ├── launch.json                    # F5 = Extension Development Host
│   └── tasks.json                     # npm: compile, wired to launch.json's preLaunchTask
├── syntaxes/
│   └── pm-lang.tmLanguage.json        # TextMate grammar
├── src/
│   ├── extension.ts                   # activate()/deactivate(), LanguageClient wiring
│   └── serverPath.ts                  # resolveServerPath() — pure, unit-tested
└── test/
    └── serverPath.test.ts
```

Root `.gitignore` gains two entries for this package's generated directories, following the
existing `/begin/node_modules` per-package convention.

---

### Task 1: Scaffold the extension project

**Files:**
- Create: `editors/vscode-pm-lang/package.json`
- Create: `editors/vscode-pm-lang/tsconfig.json`
- Create: `editors/vscode-pm-lang/language-configuration.json`
- Create: `editors/vscode-pm-lang/README.md`
- Create: `editors/vscode-pm-lang/.vscode/launch.json`
- Create: `editors/vscode-pm-lang/.vscode/tasks.json`
- Create: `editors/vscode-pm-lang/src/extension.ts`
- Modify: `.gitignore`

**Interfaces:**
- Produces: `npm run compile` (tsc), `npm run watch`, `npm test` (tsc + `node --test`) — the
  scripts every later task's steps invoke. `activate`/`deactivate` exports from
  `src/extension.ts` (no-op stubs here; Task 4 replaces the body).

- [ ] **Step 1: Create `package.json`**

```json
{
  "name": "pm-lang",
  "displayName": "pm-lang",
  "description": "Language support for pm-lang: diagnostics via the pm-lsp language server.",
  "publisher": "stlab",
  "version": "0.1.0",
  "private": true,
  "engines": {
    "vscode": "^1.85.0"
  },
  "categories": ["Programming Languages"],
  "main": "./out/src/extension.js",
  "activationEvents": [
    "onLanguage:pm-lang"
  ],
  "contributes": {
    "languages": [
      {
        "id": "pm-lang",
        "aliases": ["pm-lang", "PM"],
        "extensions": [".pm"],
        "configuration": "./language-configuration.json"
      }
    ],
    "grammars": [
      {
        "language": "pm-lang",
        "scopeName": "source.pm-lang",
        "path": "./syntaxes/pm-lang.tmLanguage.json"
      }
    ],
    "configuration": {
      "title": "pm-lang",
      "properties": {
        "pm-lang.serverPath": {
          "type": "string",
          "default": "",
          "description": "Path to the pm-lsp language server binary. If empty, the extension searches the workspace's Cargo target directory (target/debug, then target/release) and then PATH."
        }
      }
    }
  },
  "scripts": {
    "compile": "tsc -p .",
    "watch": "tsc -w -p .",
    "test": "tsc -p . && node --test out/test/"
  },
  "dependencies": {
    "vscode-languageclient": "^9.0.1"
  },
  "devDependencies": {
    "@types/node": "20.11.0",
    "@types/vscode": "1.85.0",
    "typescript": "5.7.3"
  }
}
```

- [ ] **Step 2: Create `tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "commonjs",
    "moduleResolution": "node",
    "lib": ["ES2022"],
    "outDir": "out",
    "rootDir": ".",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "sourceMap": true,
    "resolveJsonModule": true
  },
  "include": ["src/**/*.ts", "test/**/*.ts"]
}
```

- [ ] **Step 3: Create `language-configuration.json`**

```json
{
  "comments": {
    "lineComment": "//",
    "blockComment": ["/*", "*/"]
  },
  "brackets": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"]
  ],
  "autoClosingPairs": [
    { "open": "{", "close": "}" },
    { "open": "[", "close": "]" },
    { "open": "(", "close": ")" },
    { "open": "\"", "close": "\"" }
  ],
  "surroundingPairs": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
    ["\"", "\""]
  ]
}
```

- [ ] **Step 4: Create `src/extension.ts` (no-op stub — Task 4 fills in the real body)**

```typescript
import * as vscode from 'vscode';

/** Activates the pm-lang extension. */
export function activate(_context: vscode.ExtensionContext): void {}

/** Deactivates the pm-lang extension. */
export function deactivate(): void {}
```

- [ ] **Step 5: Create `.vscode/launch.json`**

Opens the repo root (two levels up from `editors/vscode-pm-lang/`) as the workspace in the
Extension Development Host, so `target/debug/pm-lsp` (built from the repo root) is where the
extension's workspace-search step expects it — see Task 5's manual verification.

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "Run Extension",
      "type": "extensionHost",
      "request": "launch",
      "args": [
        "--extensionDevelopmentPath=${workspaceFolder}",
        "${workspaceFolder}/../.."
      ],
      "outFiles": ["${workspaceFolder}/out/**/*.js"],
      "preLaunchTask": "npm: compile"
    }
  ]
}
```

- [ ] **Step 6: Create `.vscode/tasks.json`**

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "type": "npm",
      "script": "compile",
      "problemMatcher": "$tsc",
      "isBackground": false,
      "presentation": { "reveal": "silent" },
      "group": { "kind": "build", "isDefault": true }
    }
  ]
}
```

- [ ] **Step 7: Create `README.md`**

```markdown
# pm-lang for VS Code

Editor support for `.pm` (pm-lang) files: syntax highlighting and live diagnostics via the
`pm-lsp` language server.

## Requirements

Build `pm-lsp` first, from the repository root:

\`\`\`bash
cargo build -p pm-lsp
\`\`\`

The extension looks for the `pm-lsp` binary in this order:

1. The `pm-lang.serverPath` setting, if set.
2. `target/debug/pm-lsp` (or `.exe` on Windows), then `target/release/pm-lsp`, relative to the
   open workspace folder.
3. `pm-lsp` on `PATH`.

## Development

\`\`\`bash
npm install
npm run compile   # or: npm run watch
npm test          # unit tests for server-path resolution
\`\`\`

Press F5 (or Run > Start Debugging) to launch an Extension Development Host with this extension
loaded, with the repository root opened as its workspace.
```

- [ ] **Step 8: Add generated directories to the root `.gitignore`**

Read the current root `.gitignore` first, then add these two lines after the existing
`/begin/node_modules` line:

```
/editors/vscode-pm-lang/node_modules
/editors/vscode-pm-lang/out
```

- [ ] **Step 9: Install dependencies**

Run: `cd editors/vscode-pm-lang && npm install`
Expected: exits 0, creates `node_modules/` and `package-lock.json`.

- [ ] **Step 10: Verify the project compiles**

Run: `cd editors/vscode-pm-lang && npm run compile`
Expected: exits 0, creates `out/src/extension.js`, no errors printed.

- [ ] **Step 11: Commit**

```bash
git add editors/vscode-pm-lang .gitignore
git commit -m "feat(vscode-pm-lang): scaffold VS Code extension project"
```

---

### Task 2: TextMate grammar and language configuration

**Files:**
- Create: `editors/vscode-pm-lang/syntaxes/pm-lang.tmLanguage.json`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `source.pm-lang` grammar scope, referenced by `package.json`'s `contributes.grammars`
  (already wired in Task 1).

- [ ] **Step 1: Create the TextMate grammar**

Keywords, built-in primitive types, operators, comments, and numeric-suffix literals below are
all drawn directly from `pm-lang`'s grammar (`pm-lang/src/ast_parser.rs`'s `is_keyword("sheet"
| "cell" | "relationship" | "conditional" | "method")`) and `cel-parser`'s built-in type set
(`TypeRegistry::new()`'s defaults: `i8..i128`, `u8..u128`, `isize`/`usize`, `f32`/`f64`, `bool`,
`String`). Comments are plain Rust `//` / `/* */` comments, since pm-lang tokenizes via
`proc_macro2` the same as `cel-parser` (see `docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`'s
"Background / constraints" section).

```json
{
  "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
  "name": "pm-lang",
  "scopeName": "source.pm-lang",
  "fileTypes": ["pm"],
  "patterns": [
    { "include": "#comments" },
    { "include": "#keywords" },
    { "include": "#types" },
    { "include": "#strings" },
    { "include": "#numbers" },
    { "include": "#operators" }
  ],
  "repository": {
    "comments": {
      "patterns": [
        {
          "name": "comment.line.double-slash.pm-lang",
          "match": "//.*$"
        },
        {
          "name": "comment.block.pm-lang",
          "begin": "/\\*",
          "end": "\\*/"
        }
      ]
    },
    "keywords": {
      "patterns": [
        {
          "name": "keyword.declaration.pm-lang",
          "match": "\\b(sheet|cell|relationship|conditional|method)\\b"
        },
        {
          "name": "keyword.other.wildcard.pm-lang",
          "match": "(?<!\\w)_(?!\\w)"
        }
      ]
    },
    "types": {
      "patterns": [
        {
          "name": "storage.type.pm-lang",
          "match": "\\b(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|String)\\b"
        }
      ]
    },
    "strings": {
      "name": "string.quoted.double.pm-lang",
      "begin": "\"",
      "end": "\"",
      "patterns": [
        { "name": "constant.character.escape.pm-lang", "match": "\\\\." }
      ]
    },
    "numbers": {
      "patterns": [
        {
          "name": "constant.numeric.pm-lang",
          "match": "\\b\\d+(\\.\\d+)?([eE][+-]?\\d+)?(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64)?\\b"
        }
      ]
    },
    "operators": {
      "patterns": [
        {
          "name": "keyword.operator.arrow.pm-lang",
          "match": "->|=>"
        },
        {
          "name": "keyword.operator.pm-lang",
          "match": "&&|\\|\\||==|!=|<=|>=|<<|>>|[-+*/%<>=!&|^:;,]"
        }
      ]
    }
  }
}
```

- [ ] **Step 2: Verify the grammar file is valid JSON**

Run: `cd editors/vscode-pm-lang && node -e "JSON.parse(require('fs').readFileSync('syntaxes/pm-lang.tmLanguage.json', 'utf8')); console.log('ok')"`
Expected: prints `ok`.

- [ ] **Step 3: Manual verification — syntax highlighting**

No automated grammar test is added, per the design doc's Testing Strategy (VS Code extension:
manual verification checklist, no automated UI test suite for v1). Verify by hand:

1. `cd editors/vscode-pm-lang && npm run compile`.
2. Open the `editors/vscode-pm-lang` folder in VS Code, press F5 to launch the Extension
   Development Host (it opens the repo root per `.vscode/launch.json`).
3. In that new window, open `begin/assets/demo.pm`.
4. Confirm: `sheet`/`cell`/`relationship`/`conditional`/`method` are colored as keywords;
   `f64`/`i32` are colored distinctly (as types); `//` or `/* */` comments (add one temporarily
   if none are present) are colored as comments; numeric literals like `0i32`/`2.0` are colored
   as constants; `->`/`=>` are visibly distinct from other punctuation.

- [ ] **Step 4: Commit**

```bash
git add editors/vscode-pm-lang/syntaxes
git commit -m "feat(vscode-pm-lang): add pm-lang TextMate grammar"
```

---

### Task 3: Server-path resolution logic

**Files:**
- Create: `editors/vscode-pm-lang/src/serverPath.ts`
- Test: `editors/vscode-pm-lang/test/serverPath.test.ts`

**Interfaces:**
- Consumes: nothing from other tasks (pure logic, no `vscode` API).
- Produces: `resolveServerPath(options: ResolveServerPathOptions): string | undefined`, and the
  `ResolveServerPathOptions` interface — Task 4's `extension.ts` imports and calls this directly.

- [ ] **Step 1: Write the failing tests**

Create `test/serverPath.test.ts`:

```typescript
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

test('returns undefined when nothing is found anywhere', () => {
  const result = resolveServerPath(
    options({
      workspaceRoot: '/repo',
      pathEnv: '/usr/bin',
    }),
  );
  assert.equal(result, undefined);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd editors/vscode-pm-lang && npm test`
Expected: FAIL — `tsc` reports `Cannot find module '../src/serverPath'` (the file doesn't exist
yet).

- [ ] **Step 3: Implement `src/serverPath.ts`**

```typescript
import * as path from 'node:path';

/** Inputs needed to resolve the `pm-lsp` binary's filesystem location. */
export interface ResolveServerPathOptions {
  /** The user's `pm-lang.serverPath` setting, if set. */
  configuredPath: string | undefined;
  /** The first workspace folder's filesystem path, if any workspace is open. */
  workspaceRoot: string | undefined;
  /** `process.platform`, injected so tests can exercise Windows and Unix naming. */
  platform: NodeJS.Platform;
  /** `process.env.PATH`, injected so tests don't depend on the real environment. */
  pathEnv: string | undefined;
  /** Checks whether a file exists at the given path, injected so tests avoid real disk I/O. */
  fileExists: (candidate: string) => boolean;
}

const PM_LSP_UNIX = 'pm-lsp';
const PM_LSP_WINDOWS = 'pm-lsp.exe';

/** Returns the `pm-lsp` binary name for `platform` (`pm-lsp.exe` on Windows, `pm-lsp` elsewhere). */
function binaryName(platform: NodeJS.Platform): string {
  return platform === 'win32' ? PM_LSP_WINDOWS : PM_LSP_UNIX;
}

/**
 * Resolves the filesystem path of the `pm-lsp` binary to launch.
 *
 * Resolution order:
 * 1. `options.configuredPath`, if non-empty — used only if it exists; a configured path that
 *    doesn't exist is a user error and must not silently fall through to auto-detection.
 * 2. `<workspaceRoot>/target/debug/<binary>`, then `<workspaceRoot>/target/release/<binary>`.
 * 3. Each directory in `options.pathEnv` (in order), joined with `<binary>`.
 *
 * Returns `undefined` if none of the above exist.
 */
export function resolveServerPath(options: ResolveServerPathOptions): string | undefined {
  const { configuredPath, workspaceRoot, platform, pathEnv, fileExists } = options;
  const binary = binaryName(platform);

  if (configuredPath) {
    return fileExists(configuredPath) ? configuredPath : undefined;
  }

  if (workspaceRoot) {
    for (const profile of ['debug', 'release']) {
      const candidate = path.join(workspaceRoot, 'target', profile, binary);
      if (fileExists(candidate)) {
        return candidate;
      }
    }
  }

  if (pathEnv) {
    const delimiter = platform === 'win32' ? ';' : ':';
    for (const dir of pathEnv.split(delimiter)) {
      if (!dir) {
        continue;
      }
      const candidate = path.join(dir, binary);
      if (fileExists(candidate)) {
        return candidate;
      }
    }
  }

  return undefined;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd editors/vscode-pm-lang && npm test`
Expected: PASS — all 7 tests green.

- [ ] **Step 5: Commit**

```bash
git add editors/vscode-pm-lang/src/serverPath.ts editors/vscode-pm-lang/test/serverPath.test.ts
git commit -m "feat(vscode-pm-lang): add pm-lsp server-path resolution with unit tests"
```

---

### Task 4: Language client wiring

**Files:**
- Modify: `editors/vscode-pm-lang/src/extension.ts` (replace Task 1's stub body)

**Interfaces:**
- Consumes: `resolveServerPath`/`ResolveServerPathOptions` from `./serverPath` (Task 3).
- Produces: the real `activate`/`deactivate` VS Code entry points.

- [ ] **Step 1: Replace `src/extension.ts`'s contents**

```typescript
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
    vscode.window.showErrorMessage(
      'pm-lang: could not find the pm-lsp language server binary. Build it with ' +
        '"cargo build -p pm-lsp", or set the "pm-lang.serverPath" setting.',
    );
    return;
  }

  const serverOptions: ServerOptions = { command: serverPath };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'pm-lang' }],
  };

  client = new LanguageClient('pm-lang', 'pm-lang Language Server', serverOptions, clientOptions);
  context.subscriptions.push({ dispose: () => void client?.stop() });
  void client.start();
}

/** Deactivates the extension, stopping the language client if it's running. */
export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
```

- [ ] **Step 2: Verify the project still compiles**

Run: `cd editors/vscode-pm-lang && npm run compile`
Expected: exits 0, no errors.

- [ ] **Step 3: Verify unit tests still pass (unaffected by this change)**

Run: `cd editors/vscode-pm-lang && npm test`
Expected: PASS — same 7 tests as Task 3.

- [ ] **Step 4: Commit**

```bash
git add editors/vscode-pm-lang/src/extension.ts
git commit -m "feat(vscode-pm-lang): wire vscode-languageclient to the pm-lsp binary"
```

---

### Task 5: End-to-end manual verification and Phase 3 handoff

**Files:**
- Create: `docs/superpowers/2026-07-20-phase-3-vscode-extension-handoff.md`

**Interfaces:**
- Consumes: everything from Tasks 1-4 (finished extension).
- Produces: a dated handoff doc, per this repo's `CLAUDE.md` requirement to write one before
  opening a PR for a multi-step piece of work.

- [ ] **Step 1: Build `pm-lsp` from the repo root**

Run (from the repo root, not `editors/vscode-pm-lang`): `cargo build -p pm-lsp`
Expected: exits 0, produces `target/debug/pm-lsp` (`.exe` on Windows).

- [ ] **Step 2: Manual verification — diagnostics end-to-end**

1. `cd editors/vscode-pm-lang && npm run compile`.
2. Open the `editors/vscode-pm-lang` folder in VS Code, press F5. The Extension Development Host
   opens with the repo root as its workspace (per `.vscode/launch.json`).
3. In that window, create a scratch file `scratch.pm` (anywhere under the repo root) with:
   ```
   sheet s {
       cell x: i32 = 1.0;
   }
   ```
4. Confirm a diagnostic (red squiggle) appears under `1.0`, matching the type mismatch
   `pm-lsp`'s own test (`pm-lsp/src/dispatch.rs`'s
   `open_notification_triggers_a_publish_diagnostics_notification`) asserts for the same source.
5. Fix the file to `cell x: i32 = 1;` and confirm the diagnostic disappears on save/edit.
6. Delete `scratch.pm`.

- [ ] **Step 3: Manual verification — `pm-lang.serverPath` setting**

1. In the Extension Development Host window, open Settings and set `pm-lang.serverPath` to a
   nonexistent path (e.g. `/does/not/exist`).
2. Reload the window (Developer: Reload Window).
3. Confirm the "could not find the pm-lsp language server binary" error message appears (proving
   the configured-but-missing path does *not* silently fall back).
4. Clear the setting and reload again; confirm diagnostics work again (Step 2 still passes).

- [ ] **Step 4: Write the Phase 3 completion handoff**

Create `docs/superpowers/2026-07-20-phase-3-vscode-extension-handoff.md`:

```markdown
# pm-lang Language Server — Phase 3 Complete

Written after the VS Code extension sub-plan landed, closing out Phase 3 of
`docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`. See
`docs/superpowers/2026-07-18-phase-3-handoff.md` for the fuller history of how Phase 3 was split
into three sub-plans (type checking v1, `pm-lsp`, this VS Code extension) and what each one added.

## What's done

All three Phase 3 sub-plans are complete:

1. Type checking v1 (PR #45) — `cel_parser::Ty`/`check_expr`, `pm_lang::typecheck::check_sheet`.
2. `pm-lsp` (PR #46) — the language server binary, `textDocument/publishDiagnostics` over stdio.
3. **VS Code extension (`editors/vscode-pm-lang/`) — this sub-plan.** TextMate grammar for `.pm`
   syntax highlighting, `vscode-languageclient` wiring to `pm-lsp` over stdio, a
   `pm-lang.serverPath` setting (falling back to the workspace's `target/debug`/`target/release`,
   then `PATH`). First end-to-end usable milestone per the design doc's phasing: open a `.pm`
   file in VS Code, get highlighting and live diagnostics.

Full plan: `docs/superpowers/plans/2026-07-20-pm-lang-vscode-extension.md`.

## What's left (design doc's later phases, not started)

1. **Formatter** (design doc phase 4) — not started. Would add a `pm-lang` formatting module
   walking the trivia-annotated AST, wired into `pm-lsp`'s `textDocument/formatting` and the
   extension's format-on-save.
2. **Hover, go-to-definition/find-references, completion, document symbols** (design doc phase 5)
   — not started. Built on the same `AstContext`-produced AST; identifier resolution reuses the
   cell-name scoping already established by `pm_lang::parser`'s `cell_names` map concept.

## Still deliberately deferred (unchanged from the 2026-07-18 handoff)

1. `PmParser`'s grammar is still hand-duplicated against `PmAstParser`'s — the required immediate
   follow-on cleanup plan described in the 2026-07-18 handoff has not been started.
2. The known `AstContext` recovery limitation with dangling braces in CEL `if`-expressions
   ([github.com/stlab/cel-rs#43](https://github.com/stlab/cel-rs/issues/43)) is unchanged.
3. A declared-type manifest or in-source syntax for richer static checking of custom types is
   still out of scope, per the design doc.

## Key files this sub-plan added

- `editors/vscode-pm-lang/package.json` — extension manifest, `pm-lang.serverPath` setting,
  `pm-lang` language/grammar contribution.
- `editors/vscode-pm-lang/syntaxes/pm-lang.tmLanguage.json` — the TextMate grammar.
- `editors/vscode-pm-lang/src/serverPath.ts` — `resolveServerPath`, unit-tested in
  `test/serverPath.test.ts`.
- `editors/vscode-pm-lang/src/extension.ts` — `activate`/`deactivate`, the `vscode-languageclient`
  wiring.
```

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/2026-07-20-phase-3-vscode-extension-handoff.md
git commit -m "docs: add Phase 3 completion handoff for the VS Code extension"
```
