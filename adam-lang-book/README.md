# The Adam Programming Language — book

This crate holds *The Adam Programming Language*, the tutorial and reference manual for
[`adam_lang`](../adam-lang), rendered from `book-src/` via [mdBook](https://rust-lang.github.io/mdBook/).
See `src/lib.rs`'s doc comment for how the book's examples stay synchronized with real,
compiled, tested code.

Every code example in the book is also **live**: below each example's static source, the
published site mounts an editable `SheetInspector` (from
[`adam-web-ui`](../adam-web-ui)) bound to a real, running `adam_rs::Sheet` parsed from that
same source, compiled to WebAssembly. That requires a few extra pieces to be built and staged
before `mdbook` runs — a plain `mdbook build` here does not work on a fresh checkout.

## Building the book

In VS Code, run the **`book: build (full)`** or **`book: serve (full)`** task (Terminal →
Run Task…) to do everything below in one step. The commands are broken out as individual
`book: install preprocessor` / `book: build wasm` / `book: prepare assets` / `book: build` /
`book: serve` tasks too, in `.vscode/tasks.json`, if you only need to re-run one of them.

From the repository root:

```bash
# One-time, or after changing adam-lang-book-preprocessor:
cargo install --path adam-lang-book-preprocessor --force

# After changing adam-web-ui, adam-lang-book-live, or their dependencies:
cd adam-lang-book-live
wasm-pack build --target web --release
cd ..

# Stages the compiled wasm/js bundle, the Spectrum Web Components bundle
# (begin/assets/swc.js), begin/assets/inspector.css, and a generated
# manifest of every example's source into book-src/theme/:
cargo run -p xtask -- prepare-live-book-assets

mdbook build adam-lang-book
```

`book.toml` registers `mdbook-live-examples` as a preprocessor (`command =
"mdbook-live-examples"`), so `mdbook build`/`mdbook serve` fail outright if that binary isn't
on `PATH` — the first step above is not optional.

## Serving locally with live reload

Once the three staging steps above have run at least once, `mdbook serve` works normally and
picks up prose/markdown edits with live reload, same as any other mdBook site (run from the
repository root, so the `mdbook build` step above and this one share the same `book-dist/`):

```bash
mdbook serve ./adam-lang-book
```

Confirmed working: this serves the live-mount `<div>`s exactly as `mdbook build` produces them.

`mdbook serve`'s watch does **not** re-run the wasm build or the asset-staging step. Re-run
`wasm-pack build` and `cargo run -p xtask -- prepare-live-book-assets` (see above) whenever you
change `adam-web-ui`, `adam-lang-book-live`, or want an edited `.adm2` example's source picked
up by the live-mount manifest, then let `mdbook serve`'s own watch pick up the rest.

## What's generated vs. committed

`book-src/theme/` holds two hand-written, committed files (`adam-live-bootstrap.js`,
`adam-live.css`) alongside everything `prepare-live-book-assets`/`wasm-pack` generate into the
same directory (the compiled wasm/js bundle, `swc.js`, `inspector.css`, and
`adam-live-examples.json`). The directory's own `.gitignore` (`*`) keeps the generated files
out of git while leaving the two already-tracked files alone — never `git add -f` a new file
into `book-src/theme/` without checking whether it's actually meant to be committed source
versus a build artifact.

`book-dist/` (mdBook's own output directory) is gitignored at the repo root and is never
committed.

## Testing the bootstrap

From the repository root, run:

```bash
node --experimental-vm-modules --test adam-lang-book/tests/bootstrap.test.cjs
```

The tests execute the authored classic script with Node's built-in runner and VM.
They cover mount planning, independent graph lists, dependency selection, manifest
HTTP failures, script load failures, and visible page errors without installing packages.
The VM flag lets the fixture observe dynamic imports as well as fetches and script
insertions, including their absence on pages without mounts.
When a dependency or manifest fails, every affected live mount displays an accessible
error instead of remaining silently empty. Real rendered interactions still require
the staged WebAssembly and browser verification.

Each page may mount an inspector example name only once, with any number of matching
graphs. Duplicate inspector names produce a visible error before live roots mount,
even without matching graphs: the live inspector uses the example name to namespace
its DOM controls. Independent roots with the same name would address the same controls.

## CI

Both `.github/workflows/ci.yml` and `.github/workflows/docs.yml` run the same sequence of
steps above before their own `mdbook build adam-lang-book` step, so the published site (and
the PR-check build) always includes working live examples.
