# Spectrum Web Components Integration Design

**Date:** 2026-06-25
**Branch:** worktree-spectrum-supprt
**Status:** Approved — ready for implementation planning

## Overview

Integrate Adobe Spectrum Web Components (SWC) into the `begin` app as a full UI overhaul. The app is built with Dioxus 0.7.9 (a Rust-based React-inspired framework), targets both desktop (WebView2/WKWebView) and web (WASM). The current UI uses hand-rolled inline CSS throughout.

**Spectrum flavor chosen:** Spectrum Web Components — not React Spectrum (incompatible with Dioxus) and not Spectrum CSS alone (no JS behavior, requires hand-coding ARIA/keyboard navigation).

**Theme:** Spectrum 1 light (`color="light" scale="medium"`). Migrating to Spectrum 2 later is low-effort: changing the `system` attribute on `sp-theme` is the primary change; component names and props are stable across versions.

## Section 1: Asset management

### Dependency manifest

`begin/assets/versions.toml` is the canonical record of vendored JS assets:

```toml
[d3]
version = "7.9.0"
url = "https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js"
file = "d3.v7.min.js"

[spectrum-web-components]
version = "0.45.4"
url = "https://jspm.dev/npm:@spectrum-web-components/bundle@0.45.4/elements.js"
file = "swc.js"
```

### xtask crate

An `xtask` workspace member provides a cross-platform Rust task runner. Running `cargo xtask fetch-assets` reads `begin/assets/versions.toml`, downloads each entry (using `ureq`), and writes the files to `begin/assets/`. D3 is brought under the same management retroactively.

To upgrade a dependency: edit the `version` and `url` fields in `versions.toml`, then run `cargo xtask fetch-assets`.

`xtask` is dev-only and excluded from the production binary. It is added to the workspace `Cargo.toml` with `default-members` excluding it from `cargo build --workspace`.

### VS Code task

A `begin: fetch assets` task is added to `.vscode/tasks.json` under the "Begin Tasks" section:

```json
{
    "label": "begin: fetch assets",
    "type": "shell",
    "command": "cargo",
    "args": ["xtask", "fetch-assets"],
    "options": { "cwd": "${workspaceFolder}" },
    "problemMatcher": ["$rustc"],
    "presentation": { "reveal": "always", "panel": "dedicated" }
}
```

### Asset files

`begin/assets/` contains:

- `d3.v7.min.js` — vendored, managed by xtask
- `swc.js` — vendored SWC bundle, managed by xtask
- `graph.js` — authored
- `graph.css` — authored

`d3.v7.min.js` and `swc.js` are committed to the repository so the project works on a fresh clone without any setup step. `cargo xtask fetch-assets` is the mechanism for **updating** to a new version, not a required setup step. `versions.toml` is also committed.

## Section 2: App shell and theming

Every SWC component requires an `<sp-theme>` ancestor to resolve CSS custom property tokens. Without it, components render with missing colors and sizing.

In `app.rs`, the root `App` component:

1. Loads the SWC bundle with `document::Script { r#type: "module", src: asset!("/assets/swc.js") }` alongside the existing D3 script tags.
2. Wraps the entire UI in `SpTheme { color: "light", scale: "medium" }` so tokens cascade to both panels.

The two-panel flex container (`position: fixed; inset: 0; display: flex`) is unchanged structurally — it moves inside `SpTheme`. Spectrum does not provide an app-level layout component; CSS flex/grid remains the layout mechanism.

## Section 3: Inspector panel

The `Inspector` and `CellRow` components in `inspector.rs` replace raw HTML elements with SWC components. The Dioxus signal logic (`input`, `is_focused`, `has_error`, `use_effect` sync-on-blur) is unchanged — only the RSX output changes.

| Current element | SWC replacement | Notes |
| --- | --- | --- |
| `<h3>` "Cells" heading | `SpHeading` (`sp-heading`) | Section label |
| Label `<div>` | `SpFieldLabel` (`sp-field-label`) | Associates with the input via `for_` |
| Value display `<div style="color:#888">` | `<sp-body>` text or `SpHelpText` | Read-only current value below label |
| `<input type="text">` | `SpTextfield` (`sp-textfield`) | `invalid` prop replaces `border-color: #c00` |
| Cell row margin spacer | `SpDivider` (`sp-divider`) between rows | Optional but cleaner |

`sp-textfield` fires standard DOM `input`, `focus`, and `blur` events — identical to what Dioxus's `oninput`, `onfocus`, `onblur` handlers consume today. No JS bridge is needed.

The sidebar container's `font-family: monospace` is removed; SWC components use Spectrum typography. The `width: 260px` and `border-left` are replaced by a `SpDivider` on the left edge and Spectrum sizing tokens.

## Section 4: `spectrum.rs` bindings module

`begin/src/spectrum.rs` is the single place in the codebase that knows SWC element names. All other modules import from here. If a component name or attribute changes in a future SWC version, only this file changes.

Components provided:

| Dioxus component | SWC element | Key props |
| --- | --- | --- |
| `SpTheme` | `sp-theme` | `color: &str`, `scale: &str`, `children` |
| `SpTextfield` | `sp-textfield` | `value: String`, `invalid: bool`, `oninput: EventHandler<FormEvent>`, `onfocus: EventHandler<FocusEvent>`, `onblur: EventHandler<FocusEvent>` |
| `SpFieldLabel` | `sp-field-label` | `for_: String`, `children` |
| `SpDivider` | `sp-divider` | `size: Option<&str>` |
| `SpHeading` | `sp-heading` | `size: Option<&str>`, `children` |

Each is a `#[component]` function with typed props. Internally each renders the hyphenated custom element tag using Dioxus 0.7's custom element binding mechanism (element binding macros mirroring the `dioxus-html` pattern — exact API to be confirmed against Dioxus 0.7 docs during implementation).

## Out of scope

- The D3 graph view (SVG — Spectrum doesn't apply)
- Spectrum 2 migration (deferred; low-effort when ready)
- Tree-shaking / npm bundling (full SWC bundle is acceptable for a desktop dev tool)
- Dark mode (deferred)
