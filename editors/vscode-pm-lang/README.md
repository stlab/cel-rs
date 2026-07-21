# pm-lang for VS Code

Editor support for `.adm2` (pm-lang) files: syntax highlighting and live diagnostics via the
`pm-lsp` language server. (`.adm2`, not `.pm`, to avoid colliding with the `.pm` extension VS
Code already associates with Perl modules.)

## Requirements

Build `pm-lsp` first, from the repository root:

```bash
cargo build -p pm-lsp
```

The extension looks for the `pm-lsp` binary in this order:

1. The `pm-lang.serverPath` setting, if set.
2. `target/debug/pm-lsp` (or `.exe` on Windows), then `target/release/pm-lsp`, relative to the
   open workspace folder.
3. `pm-lsp` on `PATH`.

## Trying it out

The `.vscode/launch.json`/`tasks.json` in this folder only take effect when **this folder**
(`editors/vscode-pm-lang`), not the repository root, is the folder VS Code has open — that's what
makes F5 find the "Run Extension" debug config instead of just trying to run whatever file
happens to be focused.

1. Build `pm-lsp` (see Requirements above) and install/compile this extension:

   ```bash
   cd editors/vscode-pm-lang
   npm install
   npm run compile
   ```

2. Open **this folder** as its own VS Code window: `File > Open Folder...` →
   `editors/vscode-pm-lang` (a separate window from any window you have the whole `cel-rs`
   repo open in).

3. In that window, press F5 (or the Run and Debug panel's green play button, with "Run
   Extension" selected). This opens a **second** new window titled
   `[Extension Development Host]` with this extension loaded, with no folder open yet.

4. In the `[Extension Development Host]` window, use **File > Open Folder...** to open the
   repository root (`cel-rs`) — not just a single file. This matters: the extension looks for
   `pm-lsp` under `target/debug`/`target/release` *relative to the open workspace folder* (see
   Requirements above), so opening `demo.adm2` on its own, with no folder open, skips that check
   entirely and the extension won't find the binary you already built. With the repo root open as
   a folder, open `begin/assets/demo.adm2` (or any `.adm2` file) and confirm:
   - Syntax highlighting: `sheet`/`cell`/`relationship`/`conditional`/`method` are colored as
     keywords, `f64`/`i32`/etc. as types, `//` comments dimmed.
   - Live diagnostics: edit a cell's initializer to the wrong type (e.g. change
     `cell a: f64 = 2.0;` to `cell a: f64 = 2;`) — a red squiggle and a Problems-panel entry
     should appear within about a second; fixing it back makes the diagnostic disappear.
   - The `pm-lang.serverPath` setting (Settings → search "pm-lang") can override which `pm-lsp`
     binary is launched; see Requirements above for the default search order.

5. To stop, close the `[Extension Development Host]` window, or press Shift+F5 in the original
   `editors/vscode-pm-lang` window.

## Development

```bash
npm install
npm run compile   # or: npm run watch
npm test          # unit tests for server-path resolution
```
