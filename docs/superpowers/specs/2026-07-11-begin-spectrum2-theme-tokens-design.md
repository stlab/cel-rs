# Begin: Fix Spectrum 2 Theme-Token Loading Regression

**Date:** 2026-07-11
**Branch:** worktree-begin-improvements
**Status:** Approved — ready for implementation planning

## Problem

The Spectrum 2 migration (`system="spectrum-two"` on `<sp-theme>`, `versions.toml`'s
`spectrum-web-components` bundle bumped `0.45.0` → `1.12.2`) is a regression: after
it, **no Spectrum design-token CSS applies anywhere in `begin`'s UI**, including
components untouched by that migration (`sp-textfield`, `sp-divider`, `sp-heading` in
the Inspector panel). Everything renders with browser-default fonts/colors/spacing
instead of Spectrum's.

Confirmed live (via the new `verifying-begin-ui` skill, serving `begin` as a plain web
app and querying it through the DevTools Protocol):

- `document.querySelector('sp-theme').shadowRoot.adoptedStyleSheets.length === 0` —
  the theme component has adopted **zero** stylesheets.
- `getComputedStyle(document.querySelector('sp-action-button')).fontFamily` is
  `"Times New Roman"` — the browser's absolute default serif font, not even a generic
  sans-serif fallback, meaning every `var(--spectrum-*, ...)` reference in the
  component's own (correctly-loaded, verified non-empty) structural CSS is resolving
  to nothing and falling through to CSS's initial value.
- The component structural CSS itself **is** loading correctly — `sp-action-button`'s
  shadow root has 3 adopted stylesheets with 50+ rules referencing chains like
  `var(--spectrum-actionbutton-background-color-default, var(--system-action-button-background-color-default))`.
  Only the base token *definitions* (the `--spectrum-*`/`--system-*` custom properties
  themselves) are missing — nothing defines them anywhere in the document.

## Root cause

`@spectrum-web-components/theme@1.12.2`'s package `exports` map shows the actual
`--spectrum-*` token *values* for a given system/color/scale combination are not part
of the main "bundle" package at all — they live in separate modules:
`./spectrum-two/theme-{color}.js` and `./spectrum-two/scale-{scale}.js` (confirmed via
the npm registry). These are meant to be loaded as plain side-effect
`<script type="module">` tags in non-bundler setups — exactly `begin`'s setup — so
`<sp-theme>` (whose class logic *is* present in the vendored bundle, confirmed via
`customElements.get('sp-theme')`) can find and adopt them. `versions.toml`/`app.rs`
never gained these two script tags when `system="spectrum-two"` was introduced, so
`sp-theme` has nothing to adopt, regardless of the `system`/`color`/`scale` attribute
values set on it.

This is specific to the Spectrum 2 migration, not a pre-existing gap: it is the
`system="spectrum-two"` attribute (and the bundle version bump that made it valid)
that changed which token modules `<sp-theme>` needs supplied.

## Fix

### New vendored assets

Two new `versions.toml` entries, following the same pattern already used for the
zoom-icon assets on this branch — each a verified self-contained esm.sh bundle (no
external relative imports):

```toml
[spectrum-theme-light]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/theme@1.12.2/es2022/spectrum-two/theme-light.bundle.mjs"
file = "swc-theme-light.js"

[spectrum-scale-medium]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/theme@1.12.2/es2022/spectrum-two/scale-medium.bundle.mjs"
file = "swc-scale-medium.js"
```

Scope is light + medium only, matching `app.rs`'s current (and only) `SpTheme` usage
(`color: "light"`, `scale: "medium"`) — no dark theme or other scale variants are
vendored now (YAGNI; add them if/when the app actually offers a color/scale toggle).

### `app.rs` changes

Two new `document::Script { r#type: "module", ... }` tags loading the above,
alongside the existing `swc.js` tag (and the two icon script tags from the prior
plan). Load order relative to those does not matter — the officially documented
consumption pattern is a flat list of independent `<script type="module">` tags, not
a dependency-ordered chain.

### Verification

Compile-time checks (`cargo build`/`clippy`) cannot catch this class of bug — it was
invisible to them the first time. The fix's actual proof is re-running the *same*
live diagnostic that found the regression, via the `verifying-begin-ui` skill:

- `document.querySelector('sp-theme').shadowRoot.adoptedStyleSheets.length` must be
  `> 0` (was `0`).
- `getComputedStyle(document.querySelector('sp-action-button')).fontFamily` must
  reference a real Spectrum font stack (not `"Times New Roman"`).
- A screenshot of the running app should visibly show Spectrum styling (rounded
  action-group pill, real typography, Spectrum color tokens) instead of the
  browser-default rendering from the original bug report.

## Out of scope

- Dark theme / other scale variants (`theme-dark.js`, `scale-small.js`,
  `scale-large.js`) — add only when the app actually needs them.
- Any change to component logic, click handlers, or the zoom-control redesign from
  the prior plan on this branch — this is purely a missing-asset fix.
- Auditing whether Spectrum 1 (`system` unset, bundle `0.45.0`) ever worked correctly
  — not needed to fix the current regression, and not something this plan reverts to.
