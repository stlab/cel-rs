# adam-lang for VS Code

Editor support for `.adm2` (adam-lang) files: syntax highlighting and live diagnostics via the
`adam-lsp` language server.

## Requirements

Build `adam-lsp` first, from the repository root:

```bash
cargo build -p adam-lsp
```

The extension looks for the `adam-lsp` binary in this order:

1. The `adam-lang.serverPath` setting, if set.
2. `target/debug/adam-lsp` (or `.exe` on Windows), then `target/release/adam-lsp`, relative to the
   open workspace folder.
3. `adam-lsp` on `PATH`.

## Explicit modules

Configure every imported module and importing root in `adam-lang.modules`. Paths
are relative to the **first workspace folder**; multi-root workspaces use that
same base for all entries. No workspace folder is needed for ordinary standalone
single-sheet diagnostics.

```json
{
  "adam-lang.modules": {
    "constraints": "sheets/constraints.adm2",
    "app": "sheets/app.adm2"
  }
}
```

For example, `constraints.adm2` can contain
`sheet ordered(a: f64, b: f64) {}`; `app.adm2` can contain:

```text
use constraints::ordered as ord;
sheet app {
    cell x: f64;
    cell y: f64;
    relationship ord(a: x, b: y);
}
```

The extension reads only explicitly mapped files. Invalid mappings, duplicate
source files, paths outside the workspace, invalid UTF-8 and read failures show
visible errors; none become successful empty sources. File create/change/delete
events refresh mapped files, and configuration/workspace-folder changes refresh
the complete mapping. Open text takes precedence over disk snapshots until close.
Module checking never runs initializers, methods, defaults or conditional keys.
The Problems panel includes definition/use-site related locations across files.
Formatting supports imports, aliases, parameter types and named bindings, and
refuses any recovered syntax error.

### Wire contract for other clients

Send `initializationOptions.moduleSources` as exact `{path, uri, text}` records:

```json
{
  "moduleSources": [{
    "path": "constraints",
    "uri": "file:///repo/sheets/constraints.adm2",
    "text": "sheet ordered(a: f64, b: f64) {}"
  }]
}
```

`path` is an explicit `::`-separated module identity, `uri` is its absolute source
URI and `text` is the complete owned revision. Paths and URIs must each be unique.
Include roots in this collection too; the server does not infer a module identity
for an unregistered document. It checks all definitions, not an instantiated root.
Unregistered documents use the source-only checker with the same standard and
Euclid libraries. Merely adding imports to an unregistered document does not
silently opt it into guessed module ownership.

Send the same complete object in `workspace/didChangeConfiguration.settings` for
source or configuration refresh. Empty `moduleSources: []` explicitly removes
the mapping and clears removed-source diagnostics. Report a tooling read failure
as `{"moduleSourcesError":"message"}`, not an empty source or successful empty
snapshot. The server retains the last valid ownership mapping, blocks checking
with configuration diagnostics until recovery, and never silently falls back to
source-only checking for registered modules. All open edits overlay by exact URI
through refreshes; `didClose` restores the latest supplied revision.
Only initialization permits an omitted module snapshot for source-only hosts.
Every refresh must contain `moduleSources` or `moduleSourcesError`. Missing,
null, unrelated or misspelled refresh settings report a configuration error
while retaining registered ownership; they never mean implicit removal.

Checking conservatively republishes all registered/open sources so dependents
and cleared errors stay current. Independent dependency groups are checked
separately. Malformed sources retain their syntax diagnostics while the checker
still checks successfully registered peers. Sources in failed groups also show
an explicit incomplete-check status because early checker errors can block later
analysis; independent groups keep their own semantic diagnostics.
Core libraries perform no filesystem discovery,
glob imports, automatic language migration or root execution.

## Trying it out

The `.vscode/launch.json`/`tasks.json` in this folder only take effect when **this folder**
(`editors/vscode-adam-lang`), not the repository root, is the folder VS Code has open — that's what
makes F5 find the "Run Extension" debug config instead of just trying to run whatever file
happens to be focused.

1. Build `adam-lsp` (see Requirements above) and install/compile this extension:

   ```bash
   cd editors/vscode-adam-lang
   npm install
   npm run compile
   ```

2. Open **this folder** as its own VS Code window: `File > Open Folder...` →
   `editors/vscode-adam-lang` (a separate window from any window you have the whole `cel-rs`
   repo open in).

3. In that window, press F5 (or the Run and Debug panel's green play button, with "Run
   Extension" selected). This opens a **second** new window titled
   `[Extension Development Host]` with this extension loaded, with no folder open yet.

4. In the `[Extension Development Host]` window, open `begin/assets/demo.adm2` (or any `.adm2`
   file) via **File > Open File...** — not Open Folder (see note below) — and set the
   `adam-lang.serverPath` setting so the extension can find the `adam-lsp` you already built, since
   with no folder open the `target/debug`/`target/release` search step (Requirements above) never
   runs: open Settings (Ctrl+,), search "adam-lang", and set `adam-lang.serverPath` to the absolute
   path of `target/debug/adam-lsp.exe` in your checkout (e.g.
   `D:\path\to\cel-rs\target\debug\adam-lsp.exe`). Then confirm:
   - Syntax highlighting: `sheet`/`cell`/`relationship`/`conditional`/`out`/`require` are colored as
     keywords, `f64`/`i32`/etc. as types, `//` comments dimmed.
   - Live diagnostics: edit a cell's initializer to the wrong type (e.g. change
     `cell a: f64 = 2.0;` to `cell a: f64 = 2;`) — a red squiggle and a Problems-panel entry
     should appear within about a second; fixing it back makes the diagnostic disappear.
   - Format-on-save: saving the file automatically formats it using `adam-lsp`'s formatting
     capability, enabled by this extension's `editor.formatOnSave` default for adam-lang files.

   > **Don't use File > Open Folder in this window.** Switching the open folder from inside a
   > running Extension Development Host doesn't carry the `--extensionDevelopmentPath` flag
   > forward — VS Code opens a plain, ordinary window for the new folder instead, without this
   > extension loaded at all. Setting `adam-lang.serverPath` explicitly, as above, is the reliable
   > way to try this out without fighting that limitation.

5. To stop, close the `[Extension Development Host]` window, or press Shift+F5 in the original
   `editors/vscode-adam-lang` window.

## Local Install (for development)

Unlike "Trying it out" above (which only runs inside a throwaway Extension Development Host),
this installs a real, working copy of `adam-lsp` and this extension into your regular VS Code.

1. Install `adam-lsp` onto your `PATH`, from the repository root:

   ```bash
   cargo install --path adam-lsp
   ```

   This copies the binary to `~/.cargo/bin` (normally already on `PATH`), which is the last
   place `adam-lang.serverPath` resolution checks (see Requirements above) — so once it's there,
   the extension finds it automatically, in any workspace, with no `adam-lang.serverPath` setting
   needed.

2. Package and install the extension itself:

   ```bash
   cd editors/vscode-adam-lang
   npm ci
   npm run package
   npm run install-extension
   ```

   `code` must be on your `PATH` for the second command to work — if it isn't, run
   **Shell Command: Install 'code' command in PATH** from the Command Palette (Ctrl+Shift+P)
   first.

   Both steps together are also available as a single VS Code task,
   **Install adam-lsp + Extension (Local)**, runnable from the Command Palette's
   "Tasks: Run Task" — available whether you have the repository root or just this folder
   (`editors/vscode-adam-lang`) open. On Windows, this task builds/packages from a copy of
   the workspace staged outside any git worktree, to avoid a Windows-only file-locking issue
   where installing directly from a worktree blocks `git worktree remove` until the extension
   is uninstalled. On macOS/Linux, where that lock doesn't occur, use **Install adam-lsp +
   Extension (Local, in-worktree, macOS/Linux)** instead, which runs steps 1-2 above directly.

3. Reopen VS Code (or run **Developer: Reload Window**) and open any `.adm2` file — syntax
   highlighting and diagnostics should now work in ordinary windows, not just the dev host.

To pick up a code change to `adam-lsp` or the extension, re-run the relevant step above; both
commands overwrite the previous install (`cargo install` overwrites the binary by default,
and `install-extension` passes `--force`).

## Development

```bash
npm install
npm run compile   # or: npm run watch
npm test          # unit tests for server-path resolution
```
