---
name: serving-adam-lang-book
description: Use when asked to build, serve, or preview the adam-lang-book (The Adam Programming Language tutorial/reference site), or to check that a change to a book chapter, an .adm2 example, adam-web-ui, or adam-lang-book-live renders correctly - a plain `mdbook build`/`mdbook serve` fails here because the book's live examples require a preprocessor binary and a compiled wasm bundle staged first.
---

# Building and serving adam-lang-book

## Overview

`adam-lang-book` renders every code example live: below each example's static source, the
published site mounts an editable `SheetInspector` bound to a real `adam_rs::Sheet` compiled
to WebAssembly. `book.toml` registers `mdbook-live-examples` as a required preprocessor, so
`mdbook build`/`mdbook serve` fail outright unless that binary and the wasm bundle's staged
assets already exist — there is no plain-mdbook fallback path. See `adam-lang-book/README.md`
for the full explanation; this skill is the automated procedure for the same steps.

## Procedure

Run from the repository root.

1. **Install the preprocessor** (one-time, or after editing `adam-lang-book-preprocessor`):

   ```bash
   cargo install --path adam-lang-book-preprocessor --force
   ```

2. **Build the live-mount wasm bundle** (after editing `adam-web-ui`, `adam-lang-book-live`,
   or their dependencies — skip if neither has changed since the last build):

   ```bash
   cd adam-lang-book-live
   wasm-pack build --target web --release
   cd ..
   ```

   `dx build` does **not** work on this crate — it's a `cdylib` library with no Dioxus app
   entry point, not a buildable Dioxus app. Always use `wasm-pack`.

3. **Stage the generated assets** into `adam-lang-book/book-src/theme/` (the compiled
   wasm/js bundle, `begin/assets/swc.js`, `begin/assets/inspector.css`, and a manifest of
   every `.adm2` example's source — re-run this whenever an example's source changes, even
   if the wasm bundle itself didn't need rebuilding):

   ```bash
   cargo run -p xtask -- prepare-live-book-assets
   ```

4. **Build or serve:**

   - One-shot build: `mdbook build adam-lang-book`, output in
     `adam-lang-book/book-dist/` (gitignored).
   - Live-reloading local preview: `mdbook serve adam-lang-book`. This picks up prose/markdown
     edits automatically; it does **not** re-run steps 1–3, so re-run them by hand after
     editing `adam-web-ui`/`adam-lang-book-live`/an `.adm2` file's content while `mdbook
     serve` is running.

## Verifying a live example actually works

`mdbook build`/`mdbook serve` succeeding only proves the static site renders — it proves
nothing about whether a live widget actually mounts and is interactive (this project's own
rule: UI changes must be seen rendered, not just built). To check a live example in a real
browser:

1. Serve the built output (`mdbook serve adam-lang-book`, or `python3 -m http.server 8000
   --directory adam-lang-book/book-dist` after a one-shot `mdbook build`).
2. Open a chapter page that has live examples (e.g. `cells.html`, `relationships.html`,
   `tutorial.html`) and confirm an editable cell list renders below the static code block, that
   editing a value re-derives dependent cells, and that the browser console shows no errors.
3. For deeper DOM/JS introspection when a screenshot alone doesn't explain a symptom, this
   repo's `verifying-begin-ui` skill documents the same headless-Edge + CDP approach
   (`.claude/skills/verifying-begin-ui/SKILL.md`) — the browser-driving mechanics there apply
   equally to a served `book-dist/` page, not just `begin`'s own app.

## Gotchas

- `expressions.html`'s `no_standard_library` example deliberately shows **no** live widget —
  its whole point is demonstrating behavior without the standard library installed, which the
  shared live parser always installs. This is intentional (see
  `adam-lang-book-live-config`'s `NO_LIVE_MOUNT` list), not a bug.
- `book-src/theme/`'s own `.gitignore` (`*`) keeps generated assets out of git while leaving
  the two hand-written, already-tracked files (`adam-live-bootstrap.js`, `adam-live.css`)
  alone — never assume a new file appearing in that directory after this procedure is meant to
  be committed.
- If `mdbook build` fails with something like `mdbook-live-examples not found on PATH`, step 1
  above was skipped or the installed binary isn't on `PATH` in this shell.
