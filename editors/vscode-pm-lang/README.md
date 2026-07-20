# pm-lang for VS Code

Editor support for `.pm` (pm-lang) files: syntax highlighting and live diagnostics via the
`pm-lsp` language server.

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

## Development

```bash
npm install
npm run compile   # or: npm run watch
npm test          # unit tests for server-path resolution
```

Press F5 (or Run > Start Debugging) to launch an Extension Development Host with this extension
loaded, with the repository root opened as its workspace.
