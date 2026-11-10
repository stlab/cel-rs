# cel-rs

[![CI](https://github.com/stlab/cel-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/stlab/cel-rs/actions/workflows/ci.yml)
[![Docs](https://github.com/stlab/cel-rs/actions/workflows/docs.yml/badge.svg)](https://github.com/stlab/cel-rs/actions/workflows/docs.yml)

To try the property-model environment without installing anything, open
[Begin in your browser](https://stlab.github.io/cel-rs/begin/).
To build and run it yourself, see [Run Begin](#run-begin).

## Documentation

| Component | Documentation |
| --- | --- |
| Languages | [CEL language guide](https://stlab.github.io/cel-rs/cel-book/) · [Adam language guide](https://stlab.github.io/cel-rs/book/) |
| Live presentation | [Adam slides](https://stlab.github.io/cel-rs/adam-slides/) · [Authoring and preview](adam-slides/README.md) |
| CEL libraries | [cel-rs](https://stlab.github.io/cel-rs/cel_rs/) · [Runtime](https://stlab.github.io/cel-rs/cel_runtime/) · [Parser](https://stlab.github.io/cel-rs/cel_parser/) · [Macros](https://stlab.github.io/cel-rs/cel_rs_macros/) · [Standard library](https://stlab.github.io/cel-rs/cel_std/) |
| Adam libraries | [Property-model runtime](https://stlab.github.io/cel-rs/adam_rs/) · [Language](https://stlab.github.io/cel-rs/adam_lang/) · [Web UI](https://stlab.github.io/cel-rs/adam_web_ui/) |
| Dioxus integration | [Standalone consumer and host lifecycle](docs/dioxus-integration.md) |
| Adam tools | [Formatter](https://stlab.github.io/cel-rs/adam_fmt/) · [Language server](https://stlab.github.io/cel-rs/adam_lsp/) · [VS Code extension](editors/vscode-adam-lang/README.md) |
| Applications | [Begin web app](https://stlab.github.io/cel-rs/begin/) · [Begin architecture](docs/VISION.md) · [EZ Adam design](docs/superpowers/specs/2026-08-24-ez-adam-design.md) |

The [Docs workflow](.github/workflows/docs.yml) builds and checks the guides,
component APIs, live slides, and Begin on pull requests, then publishes the site on
pushes to `main`. Newly added pages become available after their changes reach
`main` and the Pages deployment finishes.

A stack-based runtime for developing domain specific languages, paired with a
recursive-descent parser for CEL (Common Expression Language) and a proc-macro crate
for compile-time CEL validation. See [docs/VISION.md](docs/VISION.md) for the
long-term direction behind each crate in this workspace.

> **Status:** this project has not been released and has no clients yet. It is in
> active development and the API may change at any time.

## Run Begin

Begin lets you explore Adam property models, inspect their cells, and change
values to see constraints propagate. Choose a bundled model under **Examples**,
or select **Open…** to load an `.adm2` file from your computer. The hosted app
uses the same browser build as the local web server; it needs no backend.

### Build and run on macOS

Use Terminal for these steps. Install [Git](https://git-scm.com/downloads/mac)
and stable [Rust through rustup](https://rustup.rs/) before building.
Node.js is not required: the repository includes the JavaScript bundles.

1. Install Apple's command-line developer tools if you don't already have them:

   ```bash
   xcode-select --install
   ```

   Finish the installer before continuing. These tools provide the native linker
   and SDK; you don't need the full Xcode application.

2. Clone the repository and enter the Begin directory:

   ```bash
   git clone https://github.com/stlab/cel-rs.git
   cd cel-rs/begin
   ```

3. Install the Dioxus CLI version used by this project:

   ```bash
   cargo install dioxus-cli --version 0.7.10 --locked
   ```

4. Build and launch the desktop app:

   ```bash
   dx serve --platform desktop
   ```

   The first build downloads and compiles dependencies, then opens the Begin
   window. Keep Terminal open while using the app. Press **Control+C** to stop it.

### Build and run on Windows

Use PowerShell on Windows, not WSL, for these steps. Before building, install
[Git for Windows](https://git-scm.com/downloads/win),
[Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
with **Desktop development with C++** (including the MSVC tools and Windows SDK),
and stable [Rust through rustup](https://rustup.rs/) with its default MSVC
toolchain. The desktop app also needs the
[Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/),
which is included with many Windows installations. Node.js is not required.

1. Open a fresh PowerShell window after installing the prerequisites, then clone
   the repository and enter the Begin directory:

   ```powershell
   git clone https://github.com/stlab/cel-rs.git
   cd cel-rs/begin
   ```

2. Install the Dioxus CLI version used by this project:

   ```powershell
   cargo install dioxus-cli --version 0.7.10 --locked
   ```

3. Build and launch the desktop app:

   ```powershell
   dx serve --platform desktop --windows-subsystem console
   ```

   The first build downloads and compiles dependencies, then opens the Begin
   window. The console subsystem keeps desktop diagnostics visible. Keep
   PowerShell open while using the app. Press **Ctrl+C** to stop it.

### Run locally in a browser

After completing the setup for your operating system, run these commands from
`cel-rs/begin` in Terminal or PowerShell:

```sh
rustup target add wasm32-unknown-unknown
dx serve --platform web --no-default-features --features web --port 8080
```

Open [localhost:8080](http://localhost:8080/) after the build finishes. The explicit
feature flags select the browser renderer instead of the default desktop
renderer. Press **Ctrl+C** in the terminal to stop the server.

### Verify the app and find errors

On launch, Begin selects **diamond** and shows its graph and **Cells** panel.
Change a cell value to see the model update, or select **image_resize** to
explore an image-sizing model.

Desktop builds reload opened files and bundled examples when you edit them on
disk. Browser builds embed the bundled examples at build time and don't watch
your filesystem. In browsers that support file handles, select **Refresh** to
reread an opened file after saving it in your editor. If **Refresh** isn't
available, select **Open…** again.

For browser builds, including the hosted app, open your browser's developer
tools and select **Console** to see parse, propagation, and file-reading errors.
Desktop builds report these errors in the launching terminal. The **cycle**
example deliberately reports a constraint-cycle error.

## Workspace layout

| Crate | Role |
| --- | --- |
| [`cel-rs`](src/lib.rs) | Root façade crate — re-exports the crates below |
| [`cel-runtime`](cel-runtime) | Core stack-based runtime; all evaluation and stack machinery |
| [`cel-parser`](cel-parser) | Recursive-descent CEL parser, lexer, and error types |
| [`cel-rs-macros`](cel-rs-macros) | Proc-macro crate for compile-time CEL expression validation |
| [`cel-euclid`](cel-euclid) | Opt-in `f64` Euclid records and affine geometry; optional Adam cell types |
| [`adam-rs`](adam-rs) | Multi-way constraint system for property models |
| [`adam-lang`](adam-lang) | DSL that expresses `adam-rs` constraint systems as source text |
| [`adam-lsp`](adam-lsp) | Language server for `adam-lang` |
| [`begin`](begin) | Dioxus-based UI application for developing property models |
| [`editors/vscode-adam-lang`](editors/vscode-adam-lang) | VS Code extension: syntax highlighting and live diagnostics for `adam-lang`, via `adam-lsp` |
| [`xtask`](xtask) | Repository automation and maintenance tasks |

## Getting started

### Prerequisites

- Rust (stable), installed via [rustup](https://rustup.rs/)

### VS Code worktree tasks

The repository does not define `worktree:` tasks. Worktree creation, removal,
and branch pruning live in each contributor's VS Code user tasks (**Tasks: Open
User Tasks**) with their scripts under `~/.vscode/scripts`, so they work across
repositories. Whatever tooling you use, create worktrees under
`.claude/worktrees/` on a `worktree-<name>` branch.

### Clone and build

```bash
git clone https://github.com/stlab/cel-rs.git
cd cel-rs
git config core.hooksPath .githooks   # activate the shared git hooks (one-time)
cargo build --workspace
cargo test --workspace
```

### Add cel-rs to your project

```bash
cargo add cel-rs
```

### A first expression

```rust
use cel_rs::runtime::Segment;

// Build a segment that takes a u32 and a &str as arguments.
let segment = Segment::<(u32, &str)>::new()
    .op1r(|s| {
        let r = s.parse::<u32>()?;
        Ok(r)
    })
    .op2(|a, b| a + b)
    .op1(|r| r.to_string());
assert_eq!(segment.call((1u32, "2")).unwrap(), "3");
```

This builds a small pipeline over `cel-runtime`'s typed stack: parse the `&str` into
a `u32` (fallibly, via `op1r`), add it to the `u32` argument, then format the result.
`cel-parser` and `cel-rs-macros` cover the other two ways to produce a segment: parsing
CEL source at runtime, and validating/compiling CEL expressions at compile time. See
[`src/lib.rs`](src/lib.rs) for one example of each.

## Development

### Optional geometry

Install `cel_euclid::install(&mut lookup)` alongside `cel_std::install` to use
the [complete geometry signature table](cel-euclid/SIGNATURES.md).
For Adam, enable `cel-euclid/adam` and register cell types with
`cel_euclid::register_adam_types(&mut types)`. The web UI, LSP, and runnable
book support configure equivalent environments; `adam-lang` itself remains
geometry-independent.

```bash
cargo run -p cel-euclid --features adam --example adam_geometry
cargo test -p cel-euclid --all-features
```

The actual [crop/straighten sheet](cel-euclid/examples/crop_straighten.adm2)
fits arbitrary affine geometry with general operations and typed IIFEs,
preserves source aspiration through Adam self-reference, and derives a
fixed-overlay presentation. Its numerical policy lives in CEL, not a Rust
fitting helper. The [pan/zoom sheet](cel-euclid/examples/pan_zoom.adm2)
demonstrates graph policy. The live graph uses an Adam-owned model and a shared
Dioxus proposal/approval bridge in Begin and the book; see
[graph view ownership and verification](docs/graph-view.md).

See [CLAUDE.md](CLAUDE.md) for the full command reference (build, test, lint,
sanitizers) and repository conventions, including the requirement to work in a
separate git worktree for any change.

### Run begin's JavaScript tests

The `begin` Rust build doesn't require Node.js or npm. Use Node.js and npm only to
run its JavaScript tests. From the repository root, run:

```bash
cd begin
npm ci
npm test
```

### adam-lang: VS Code extension

`editors/vscode-adam-lang` is a VS Code extension providing syntax highlighting and live
diagnostics for `.adm2` (adam-lang) files, backed by the `adam-lsp` language server.

To install it locally:

```bash
cargo install --path adam-lsp
cd editors/vscode-adam-lang
npm ci
npm run package
npm run install-extension
```

Both the cargo install and the three npm steps are also available as a single VS Code task,
**Install adam-lsp + Extension (Local)**, runnable from the Command Palette's "Tasks: Run
Task" with either the repository root or `editors/vscode-adam-lang` open. On Windows, this
task builds/packages from a copy of the workspace staged outside any git worktree (under
`%LOCALAPPDATA%\cel-rs\adam-lsp-install`), since installing directly from a worktree leaves
the running LSP binary/extension referencing files inside it, which blocks `git worktree
remove` until the extension is uninstalled — a Windows-only file-locking issue. On
macOS/Linux, where that lock doesn't occur, use **Install adam-lsp + Extension (Local,
in-worktree, macOS/Linux)** instead, which runs the four steps above directly. See
[editors/vscode-adam-lang/README.md](editors/vscode-adam-lang/README.md) for the full
Extension Development Host walkthrough and troubleshooting notes.
