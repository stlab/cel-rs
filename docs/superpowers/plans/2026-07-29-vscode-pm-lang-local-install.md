# pm-lang VS Code Extension — Local Install for Development — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a developer install a real, working copy of `pm-lsp` and the `pm-lang` extension into their regular VS Code (not just the Extension Development Host), via `cargo install` + a packaged `.vsix`.

**Architecture:** `pm-lsp` is installed onto `PATH` via `cargo install --path pm-lsp`, which the extension's existing PATH-fallback resolution (`src/serverPath.ts`) already picks up with no configuration. The extension itself is packaged into a `.vsix` with `@vscode/vsce` and installed with `code --install-extension`. A combined VS Code task chains both.

**Tech Stack:** Cargo (existing `pm-lsp` bin crate), npm, `@vscode/vsce` 3.9.2, VS Code tasks.json.

## Global Constraints

- `@vscode/vsce` devDependency version: `^3.9.2`.
- `.vscodeignore` patterns **must not** have a leading `/` (e.g. `src/**`, not `/src/**`). `@vscode/vsce@3.9.2` matches each pattern against collected file paths (which never carry a leading `/`) via plain `minimatch`, not gitignore semantics — a leading `/` in the pattern requires the compared string to also start with `/`, which none of vsce's paths do, so a rooted pattern silently matches nothing and the dev-only files it was meant to exclude (`src/`, `test/`, `.vscode/`, `tsconfig.json`) ship in the vsix anyway. Unrooted patterns are safe here specifically because this matcher anchors each pattern to the start of the string with no gitignore-style "at any depth" fallback, so `src/**` matches only paths literally starting with `src/` and never leaks into `out/src/**` — verified by reproduction during Task 1 (see the corrected `.vscodeignore` content there).
- The `package`/`install-extension` npm scripts use a fixed output filename (`pm-lang.vsix`, via `vsce package --out pm-lang.vsix`), not the version-derived default name, so `install-extension` never needs to glob for the current version.
- `pm-lsp`'s `cargo install` task's shell `cwd` must resolve to the repo root, not `editors/vscode-pm-lang` — VS Code's `${workspaceFolder}` in this folder's own `.vscode/tasks.json` is `editors/vscode-pm-lang` itself (this project's existing F5 workflow opens that folder on its own, not the repo root), so the repo root is `${workspaceFolder}/../..`.
- **Known verification caveat:** `vsce package`'s default dependency-bundling step shells out to `npm list --production ...` as a child process. In some sandboxed/agentic shell environments (confirmed during this plan's design work), that specific nested child-process spawn causes the packaging step to silently collect zero files (every file, including the entrypoint, is reported "missing") even though the identical `npm list`/glob logic works correctly in isolation, and even with sandboxing explicitly disabled — this reproduces the same way whether invoked via PowerShell, Bash, or a raw `cmd.exe` batch file, and appears specific to nested-process stdio handling in that kind of wrapped shell, not to `vsce`, `glob`, or this project's configuration. If `npm run package` reports `ERROR Extension entrypoint(s) missing` despite the files genuinely existing on disk, retry it from a normal, non-agent-wrapped terminal (a real terminal window, or the VS Code integrated terminal run directly, not through an automation harness) before assuming the packaging config is wrong.

---

### Task 1: Extension packaging tooling

**Files:**

- Modify: `editors/vscode-pm-lang/package.json`
- Create: `editors/vscode-pm-lang/.vscodeignore`
- Modify: `.gitignore` (repo root)

**Interfaces:**

- Produces: npm scripts `package` (runs `vsce package --out pm-lang.vsix`) and `install-extension` (runs `code --install-extension pm-lang.vsix --force`), consumed by Task 2's VS Code tasks and Task 3's README.

- [ ] **Step 1: Add the `@vscode/vsce` devDependency and the packaging scripts**

Edit `editors/vscode-pm-lang/package.json`'s `scripts` and `devDependencies`:

```json
  "scripts": {
    "vscode:prepublish": "npm run compile",
    "compile": "tsc -p .",
    "watch": "tsc -w -p .",
    "test": "tsc -p . && node --test out/test/*.test.js",
    "package": "vsce package --out pm-lang.vsix",
    "install-extension": "code --install-extension pm-lang.vsix --force"
  },
  "dependencies": {
    "vscode-languageclient": "^9.0.1"
  },
  "devDependencies": {
    "@types/node": "20.11.0",
    "@types/vscode": "1.85.0",
    "@vscode/vsce": "^3.9.2",
    "typescript": "5.7.3"
  }
```

`vscode:prepublish` is `vsce`'s standard pre-package hook — `vsce package` runs it automatically before collecting files, so the vsix always ships freshly-compiled `out/`.

- [ ] **Step 2: Install the new devDependency**

Run (from `editors/vscode-pm-lang`):

```bash
npm install
```

Expected: `package-lock.json` updates to include `@vscode/vsce` and its transitive dependencies; `node_modules/@vscode/vsce` exists afterward.

- [ ] **Step 3: Create `.vscodeignore` with rooted patterns**

Create `editors/vscode-pm-lang/.vscodeignore`:

```text
.vscode/**
src/**
test/**
out/test/**
tsconfig.json
package-lock.json
**/*.map
```

None of these patterns are rooted with a leading `/`. This is required, not stylistic: `@vscode/vsce` matches `.vscodeignore` patterns against collected file paths using plain `minimatch`, not gitignore semantics, and those paths never carry a leading `/` (e.g. `src/extension.ts`, `tsconfig.json`). A rooted pattern like `/src/**` requires the compared string to also start with `/`, which none of them do — so a rooted pattern silently excludes nothing, and the dev-only files it was meant to strip (`src/`, `test/`, `.vscode/`, `tsconfig.json`) ship in the vsix anyway. Unrooted patterns are safe from leaking into `out/src/**` specifically because this matcher has no gitignore-style "match at any depth" fallback: `src/**` matches only paths that literally start with `src/`.

- [ ] **Step 4: Ignore the packaged vsix in git**

Edit the repo-root `.gitignore`, next to the existing `vscode-pm-lang` entries:

```text
/editors/vscode-pm-lang/node_modules
/editors/vscode-pm-lang/out
/editors/vscode-pm-lang/*.vsix
```

(Only the third line is new — the first two already exist.)

- [ ] **Step 5: Run the packaging script and verify the vsix**

From `editors/vscode-pm-lang`:

```bash
npm run package
```

Expected: `pm-lang.vsix` is created in `editors/vscode-pm-lang`. If it instead fails with `ERROR Extension entrypoint(s) missing`, see the Global Constraints verification caveat above before concluding the config is wrong — retry from a plain terminal.

Once it succeeds, confirm the vsix's contents exclude dev-only files and include the compiled runtime. On Windows PowerShell (a `.vsix` is a zip file):

```powershell
Copy-Item pm-lang.vsix pm-lang.vsix.zip
Expand-Archive pm-lang.vsix.zip -DestinationPath vsix-inspect -Force
Get-ChildItem -Recurse vsix-inspect/extension | Select-Object FullName
Remove-Item pm-lang.vsix.zip, vsix-inspect -Recurse -Force
```

Expected in the listing: `out/src/extension.js` and `out/src/serverPath.js` present; `src/`, `test/`, `out/test/`, `.vscode/`, `tsconfig.json`, `package-lock.json`, and any `*.map` file absent.

- [ ] **Step 6: Commit**

```bash
git add editors/vscode-pm-lang/package.json editors/vscode-pm-lang/package-lock.json editors/vscode-pm-lang/.vscodeignore .gitignore
git commit -m "feat(vscode-pm-lang): add vsce packaging scripts for local extension install"
```

(Don't add the generated `pm-lang.vsix` itself — it's now gitignored.)

---

### Task 2: VS Code task to install both locally

**Files:**

- Modify: `editors/vscode-pm-lang/.vscode/tasks.json`

**Interfaces:**

- Consumes: the `package` and `install-extension` npm scripts from Task 1.

- [ ] **Step 1: Add the cargo, packaging, and combined tasks**

Edit `editors/vscode-pm-lang/.vscode/tasks.json` to add four tasks after the existing `compile` task:

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
    },
    {
      "label": "cargo: install pm-lsp",
      "type": "shell",
      "command": "cargo",
      "args": ["install", "--path", "pm-lsp"],
      "options": { "cwd": "${workspaceFolder}/../.." },
      "presentation": { "reveal": "always", "panel": "shared" },
      "problemMatcher": []
    },
    {
      "label": "npm: package",
      "type": "npm",
      "script": "package",
      "presentation": { "reveal": "always", "panel": "shared" },
      "problemMatcher": []
    },
    {
      "label": "npm: install-extension",
      "type": "npm",
      "script": "install-extension",
      "presentation": { "reveal": "always", "panel": "shared" },
      "problemMatcher": []
    },
    {
      "label": "Install pm-lsp + Extension (Local)",
      "dependsOrder": "sequence",
      "dependsOn": ["cargo: install pm-lsp", "npm: package", "npm: install-extension"],
      "problemMatcher": []
    }
  ]
}
```

- [ ] **Step 2: Verify the task's shell commands directly**

VS Code tasks can't be driven from a shell script, so verify the same commands the task runs, in the same order, from a plain terminal. Run the first command from the repo root, the rest from `editors/vscode-pm-lang`:

```bash
cargo install --path pm-lsp   # from repo root
cd editors/vscode-pm-lang
npm run package
npm run install-extension
```

Expected: all three complete without error (`pm-lsp`/`pm-lsp.exe` reports "Installed package"; `pm-lang.vsix` is (re)created; `code --install-extension` reports success).

- [ ] **Step 3: Commit**

```bash
git add editors/vscode-pm-lang/.vscode/tasks.json
git commit -m "feat(vscode-pm-lang): add combined VS Code task to install pm-lsp + extension locally"
```

---

### Task 3: README — Local Install section

**Files:**

- Modify: `editors/vscode-pm-lang/README.md`

- [ ] **Step 1: Add the section**

Insert a new section in `editors/vscode-pm-lang/README.md`, after "Trying it out" and before "Development":

```markdown
## Local Install (for development)

Unlike "Trying it out" above (which only runs inside a throwaway Extension Development Host),
this installs a real, working copy of `pm-lsp` and this extension into your regular VS Code.

1. Install `pm-lsp` onto your `PATH`, from the repository root:

   ```bash
   cargo install --path pm-lsp
   ```

   This copies the binary to `~/.cargo/bin` (normally already on `PATH`), which is the last
   place `pm-lang.serverPath` resolution checks (see Requirements above) — so once it's there,
   the extension finds it automatically, in any workspace, with no `pm-lang.serverPath` setting
   needed.

2. Package and install the extension itself:

   ```bash
   cd editors/vscode-pm-lang
   npm install
   npm run package
   npm run install-extension
   ```

   `code` must be on your `PATH` for the second command to work — if it isn't, run
   **Shell Command: Install 'code' command in PATH** from the Command Palette (Ctrl+Shift+P)
   first.

   Both steps together are also available as a single VS Code task,
   **Install pm-lsp + Extension (Local)**, runnable from the Command Palette's
   "Tasks: Run Task" while this folder (`editors/vscode-pm-lang`) is open.

3. Reopen VS Code (or run **Developer: Reload Window**) and open any `.adm2` file — syntax
   highlighting and diagnostics should now work in ordinary windows, not just the dev host.

To pick up a code change to `pm-lsp` or the extension, re-run the relevant step above; both
commands overwrite the previous install (`cargo install` overwrites the binary by default,
and `install-extension` passes `--force`).
```

- [ ] **Step 2: Commit**

```bash
git add editors/vscode-pm-lang/README.md
git commit -m "docs(vscode-pm-lang): document local install for development"
```

---

### Task 4: End-to-end verification

**Files:**

- None (verification only).

- [ ] **Step 1: Verify `pm-lsp` resolves via PATH with no workspace open**

```bash
cargo install --path pm-lsp   # from repo root
where pm-lsp.exe   # Windows; use `which pm-lsp` on macOS/Linux
```

Expected: prints a path under `~/.cargo/bin` (or your Cargo home's `bin` dir).

- [ ] **Step 2: Verify the extension installs and appears**

```bash
cd editors/vscode-pm-lang
npm run package
npm run install-extension
code --list-extensions
```

Expected: `stlab.pm-lang` appears in the list. If `npm run package` fails with `ERROR Extension entrypoint(s) missing`, re-read the Global Constraints verification caveat and retry in a plain terminal before treating it as a real failure.

- [ ] **Step 3: Confirm the extension works in a normal (non-dev-host) window**

Open a plain VS Code window with no folder open, or any unrelated folder, and open `begin/assets/demo.adm2` (or any `.adm2` file) directly via File > Open File. Confirm:

- Syntax highlighting: `sheet`/`cell`/`relationship`/`conditional`/`method` colored as keywords, `f64`/`i32`/etc. as types, `//` comments dimmed.
- Live diagnostics: changing `cell a: f64 = 2.0;` to `cell a: f64 = 2;` produces a red squiggle and a Problems-panel entry within about a second; reverting it removes the diagnostic.

This confirms PATH-based `pm-lsp` resolution is actually what's firing (there's no workspace-relative `target/` to fall back on here).

- [ ] **Step 4: Regression-check the existing test suite**

```bash
cd editors/vscode-pm-lang
npm test
```

Expected: all existing `serverPath.test.ts` tests still pass — this project adds no new resolution logic, only packaging/build tooling around it.

- [ ] **Step 5: Run the combined task**

From the Command Palette (with `editors/vscode-pm-lang` open as the VS Code folder), run "Tasks: Run Task" → **Install pm-lsp + Extension (Local)**. Expected: all three sub-tasks run in order and complete without error.
