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
`<sp-theme>`'s class logic needs one of each loaded to have anything to adopt.

**First fix attempt (superseded — see "Rejected approach" below) tried vendoring
these as two more self-contained esm.sh `.bundle.mjs` files, mirroring the pattern
already used for the zoom-icon assets. It failed**, and the failure revealed a second,
more fundamental problem that this revision addresses:

Each self-contained `.bundle.mjs` file is rolled up by esm.sh in isolation, inlining
its *own private copy* of every module it touches — including Spectrum's `Theme` base
class. `swc.js` (the main "bundle" package, also fetched as a self-contained
`.bundle.mjs`) has its *own* private `Theme` class copy; a separately-bundled
`theme-light.bundle.mjs` has a *different, unrelated* private copy. `theme-light`'s
`registerThemeFragment(...)` call registers tokens onto its own isolated copy — never
onto the `Theme` class that `swc.js` actually instantiates as the real `<sp-theme>`
element. Confirmed live: after vendoring and wiring both new self-contained files,
`adoptedStyleSheets.length` was still `0` and the font was still `"Times New Roman"` —
byte-identical symptom to before the fix.

This is specific to Spectrum 2 theming's *fragment-registration* mechanism (module-scoped
class state), not a general problem with the self-contained-bundle vendoring pattern —
it's why the zoom icons (`customElements.define`, a genuine global browser registry,
no module-scoped state involved) vendor and work correctly as isolated self-contained
files, while theme tokens do not.

### Why the fix must change how `elements.js` is loaded too, not just the two new files

Verified directly: esm.sh's *plain* (non-`.bundle.mjs`) `theme-light.js` imports
`Theme.mjs` from `/@spectrum-web-components/theme@1.12.2/es2022/src/Theme.mjs`, and
the plain `elements.js`'s theme chain (`sp-theme.mjs` → `Theme.mjs`) resolves to that
*exact same absolute URL*. Browsers deduplicate ES modules by final resolved URL, not
by which bundle requested them — so loading the plain (unbundled) variants as
separate `<script type="module">` tags lets the browser's own module cache share one
real `Theme` instance across all of them, which is what fragment registration
actually needs. This only works if the *main* elements script is also the plain,
unbundled `elements.js` — the self-contained `swc.js` (`.bundle.mjs`) it currently is
has its own sealed `Theme` copy no external file can ever reach.

## Fix

Load Spectrum's theme-dependent assets live from esm.sh instead of vendoring them,
accepting a runtime network dependency in exchange for correct module sharing without
new tooling. (A `[patch]`-free, fully offline build via `npm install` + a real bundler
like esbuild — which dedupes shared dependencies in one pass instead of relying on
per-request bundling — is a reasonable future improvement, revisited only once adding
npm to the toolchain is acceptable; out of scope here.)

### `app.rs` changes

Replace the vendored `swc.js` script tag with a live reference to the plain elements
entry point, and add two new live script tags for the theme-token modules:

```rust
document::Script { r#type: "module", src: "https://esm.sh/@spectrum-web-components/bundle@1.12.2/elements.js" }
document::Script { r#type: "module", src: "https://esm.sh/@spectrum-web-components/theme@1.12.2/spectrum-two/theme-light.js" }
document::Script { r#type: "module", src: "https://esm.sh/@spectrum-web-components/theme@1.12.2/spectrum-two/scale-medium.js" }
document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-in.js") }
document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-out.js") }
```

The zoom-icon tags are unchanged and stay vendored — no module-sharing problem
applies to them (see Root Cause). D3 also stays vendored — unrelated to Spectrum
theming entirely.

Scope is light + medium only, matching `app.rs`'s current (and only) `SpTheme` usage
— no dark theme or other scale variants are added now (YAGNI).

### `versions.toml` / vendored-file cleanup

- Remove the `[spectrum-web-components]` entry and `begin/assets/swc.js` — no longer
  vendored, replaced by the live reference above.
- Remove the `[spectrum-theme-light]` / `[spectrum-scale-medium]` entries and their
  `.js` files added by the superseded first fix attempt — the `.bundle.mjs` variant
  they point at is exactly the kind of isolated bundle that doesn't work; they're
  replaced by the live plain-module references above, not re-vendored in a different
  form.
- `[spectrum-icon-zoom-in]` / `[spectrum-icon-zoom-out]` / `[d3]` are unchanged.

### Verification

Compile-time checks (`cargo build`/`clippy`) cannot catch this class of bug — it was
invisible to them both times. The fix's actual proof is re-running the *same* live
diagnostic that found the regression and that caught the first fix attempt's failure,
via the `verifying-begin-ui` skill:

- `document.querySelector('sp-theme').shadowRoot.adoptedStyleSheets.length` must be
  `> 0` (was `0` both before any fix and after the superseded first attempt).
- `getComputedStyle(document.querySelector('sp-action-button')).fontFamily` must
  reference a real Spectrum font stack (not `"Times New Roman"`).
- A screenshot of the running app should visibly show Spectrum styling (rounded
  action-group pill, real typography, Spectrum color tokens).

## Rejected approach

Vendoring `theme-light`/`scale-medium` as additional isolated self-contained
`.bundle.mjs` files (mirroring the zoom-icon pattern). Implemented, live-verified, and
found not to work — see Root Cause. Recorded here so a future session doesn't
re-attempt the same fix without re-deriving why it fails.

## Out of scope

- Dark theme / other scale variants — add only when the app actually needs them.
- A fully offline (npm + real bundler) rebuild of this fix — real future improvement,
  not attempted now (no npm in the toolchain yet).
- Any change to component logic, click handlers, or the zoom-control redesign from
  the prior plan on this branch — this is purely an asset-loading fix.
- Auditing whether Spectrum 1 (`system` unset, bundle `0.45.0`) ever worked correctly
  — not needed to fix the current regression, and not something this plan reverts to.
