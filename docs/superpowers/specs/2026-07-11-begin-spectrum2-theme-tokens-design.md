# Begin: Fix Spectrum 2 Theme-Token Loading Regression

**Date:** 2026-07-11 (revised 2026-07-13)
**Branch:** worktree-begin-improvements
**Status:** Approved — ready for implementation planning

## Problem

The Spectrum 2 migration (`system="spectrum-two"` on `<sp-theme>`, `versions.toml`'s
`spectrum-web-components` bundle bumped `0.45.0` → `1.12.2`) is a regression: after
it, **no Spectrum design-token CSS applies anywhere in `begin`'s UI**, including
components untouched by that migration (`sp-textfield`, `sp-divider`, `sp-heading` in
the Inspector panel). Everything renders with browser-default fonts/colors/spacing
instead of Spectrum's.

Confirmed live (via the `verifying-begin-ui` skill, serving `begin` as a plain web app
and querying it through the DevTools Protocol):

- `document.querySelector('sp-theme').shadowRoot.adoptedStyleSheets.length === 0` —
  the theme component has adopted **zero** stylesheets.
- `getComputedStyle(document.querySelector('sp-action-button')).fontFamily` is
  `"Times New Roman"` — the browser's absolute default serif font, meaning every
  `var(--spectrum-*, ...)` reference in the component's own (correctly-loaded,
  verified non-empty) structural CSS resolves to nothing and falls through to CSS's
  initial value.
- The component structural CSS itself **is** loading correctly — `sp-action-button`'s
  shadow root has 3 adopted stylesheets with 50+ rules referencing chains like
  `var(--spectrum-actionbutton-background-color-default, var(--system-action-button-background-color-default))`.
  Only the base token *definitions* are missing — nothing defines them anywhere.

## Root cause

`@spectrum-web-components/theme@1.12.2`'s package `exports` map shows the actual
`--spectrum-*` token *values* for a given system/color/scale combination live in
separate modules: `./spectrum-two/theme-{color}.js` and `./spectrum-two/scale-{scale}.js`.
`<sp-theme>`'s class logic needs one of each loaded to have anything to adopt, and
neither was ever wired up when `system="spectrum-two"` was introduced.

Fixing this took three attempts; the first two are recorded under "Rejected
approaches" so a future session doesn't re-derive the same dead ends from scratch.

## Fix: bundle Spectrum with esbuild via npm

Add a small npm-managed build step (esbuild, run manually/occasionally, output
committed like every other vendored asset) that combines `begin`'s entire Spectrum
Web Components surface — elements, Spectrum 2 theme tokens, and the zoom-control
icons — into one self-contained `begin/assets/swc.js`, replacing five separate
vendored/live script tags with one. **Verified working** with a standalone spike
(esbuild 0.24.2, Node v26.4.0): `themeAdoptedSheets` went from `0`/unreachable to `3`,
and `getComputedStyle(...).fontFamily` returned the real Spectrum stack
(`adobe-clean, "Adobe Clean", "Source Sans Pro", ...`) instead of `"Times New Roman"`;
a screenshot showed the correctly-styled rounded action-group pill with real icons.

### Why this works where the first two attempts didn't

A real bundler resolves the *entire* import graph in one compilation pass instead of
per-file/per-request:

- **Attempt 1** (vendor `theme-light`/`scale-medium` as separate self-contained esm.sh
  `.bundle.mjs` files) failed because each is rolled up by esm.sh in isolation, so each
  inlines its own private copy of Spectrum's `Theme` base class. `theme-light`'s
  `registerThemeFragment(...)` call registered tokens onto its own isolated copy —
  never the `Theme` class the real `<sp-theme>` instance (from the separately-bundled
  `swc.js`) uses.
- **Attempt 2** (load esm.sh's *plain*, unbundled modules live, relying on the
  browser's own ES module cache to dedupe the shared `Theme.mjs` dependency by
  resolved URL) failed for an unrelated reason: that unbundled module graph contains
  static `import` statements pulling in raw `.css` files served as `content-type:
  text/css`. Browsers reject non-JS MIME types in static ESM module imports, aborting
  the whole graph before a single `customElements.define()` runs.
- **esbuild**, given one entry file that imports everything, produces a single output
  where all references to `Theme` resolve to the *same* compiled class (fixing
  attempt 1's problem) and where CSS imports are handled by its own loader at build
  time, never reaching the browser as a raw static import (fixing attempt 2's
  problem). One mechanism, both problems solved — because both problems were really
  the same underlying issue: per-file/per-request bundling doesn't compile a shared
  graph, and only a real bundler does.

### Toolchain: npm, scoped narrowly

- `begin/package.json` (+ committed `package-lock.json`) lists exact-pinned
  devDependencies: `esbuild`, `@spectrum-web-components/bundle`,
  `@spectrum-web-components/theme`, `@spectrum-web-components/icons-workflow` (all
  `1.12.2` except esbuild, matching the precision-pinning style already used in
  `versions.toml`).
- `begin/js/spectrum-entry.js` — one file, five side-effect imports (elements,
  theme-light, scale-medium, both zoom icons).
- `npm run build` → `esbuild js/spectrum-entry.js --bundle --format=esm
  --outfile=assets/swc.js --loader:.css=css`.
- `cargo xtask build-js` wraps `npm ci && npm run build`, run from the repo root —
  same spirit as `cargo xtask fetch-assets`: a manual, occasional step whose *output*
  is committed, not part of the default build. Contributors who never touch the
  Spectrum integration never need Node installed. `begin/node_modules/` is
  gitignored; `package.json`/`package-lock.json`/`js/spectrum-entry.js` are committed.
- No change to Dioxus's hot-reload: `dx serve` already watches `begin/assets/` for
  changes (the same mechanism that picks up `graph.css`/`graph.js` edits today) and
  doesn't care how `swc.js` was produced.

### `app.rs` changes

Five script tags collapse to one:

```rust
document::Script { r#type: "module", src: asset!("/assets/swc.js") }
```

(Replacing the current `swc.js` + `swc-icon-zoom-in.js` + `swc-icon-zoom-out.js` trio;
the previous plan's live-esm.sh attempt never got committed.)

### `versions.toml` / vendored-file cleanup

Remove the `[spectrum-web-components]`, `[spectrum-icon-zoom-in]`,
`[spectrum-icon-zoom-out]`, `[spectrum-theme-light]`, `[spectrum-scale-medium]`
entries and their now-superseded files — everything they vendored is now produced by
`cargo xtask build-js` into the single `swc.js`. Only `[d3]` remains in
`versions.toml` (unrelated to Spectrum, unaffected by any of this).

### Documentation

Add a section to the root `README.md` covering: Node.js/npm as an occasional
prerequisite (only for rebuilding the Spectrum bundle, not everyday `begin`
development), and the exact commands (`cargo xtask build-js`, or `cd begin && npm ci
&& npm run build`) to regenerate `begin/assets/swc.js` after bumping a Spectrum
version in `begin/package.json`.

### Verification

Compile-time checks (`cargo build`/`clippy`) cannot catch this class of bug — it was
invisible to them all three times. The fix's actual proof, beyond the standalone spike
already run, is re-running the *same* live diagnostic inside the real `begin` app via
the `verifying-begin-ui` skill:

- `document.querySelector('sp-theme').shadowRoot.adoptedStyleSheets.length` must be
  `> 0`.
- `getComputedStyle(document.querySelector('sp-action-button')).fontFamily` must
  reference a real Spectrum font stack (not `"Times New Roman"`).
- A screenshot of the running app should visibly show Spectrum styling.

Scope is light + medium only, matching `app.rs`'s current (and only) `SpTheme` usage
— no dark theme or other scale variants are added now (YAGNI).

## Rejected approaches

1. **Vendor `theme-light`/`scale-medium` as additional isolated self-contained
   `.bundle.mjs` files** (mirroring the zoom-icon pattern). Implemented, live-verified,
   found not to work: module-scoped `Theme` class state can't be shared across
   independently-bundled files (see Root Cause).
2. **Load esm.sh's plain/unbundled modules live**, relying on the browser's native ES
   module cache to dedupe the shared `Theme.mjs` dependency. Implemented,
   live-verified, found not to work: that module graph contains raw `.css` static
   imports, which browsers reject as an invalid MIME type in a JS module context.

## Out of scope

- Dark theme / other scale variants — add only when the app actually needs them.
- Any change to component logic, click handlers, or the zoom-control redesign from
  the prior plan on this branch — this is purely a build/asset-pipeline fix.
- Auditing whether Spectrum 1 (`system` unset, bundle `0.45.0`) ever worked correctly
  — not needed to fix the current regression, and not something this plan reverts to.
- Minifying/optimizing the esbuild output, wiring `--watch` mode into `dx serve`, or
  any other esbuild feature beyond what the verified spike used.
