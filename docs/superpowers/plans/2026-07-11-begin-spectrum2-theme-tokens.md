# Begin: Fix Spectrum 2 Theme-Token Loading Regression Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Spectrum 2 migration regression where no Spectrum design-token CSS
applies anywhere in `begin`'s UI, by loading Spectrum's theme-dependent assets live
from esm.sh (instead of as isolated self-contained vendored bundles, which a prior
attempt on this same branch proved doesn't work for theme tokens specifically).

**Architecture:** Replace the vendored `swc.js` script tag in `app.rs` with a live
reference to esm.sh's *plain* (non-bundled) `elements.js`, and add two new live
script tags for the plain `theme-light.js`/`scale-medium.js` modules. These three
share their `Theme` class dependency via the browser's native ES module cache
(verified: they resolve it to the identical absolute esm.sh URL), which self-contained
per-file bundling cannot provide. Then remove the now-dead vendored files and
`versions.toml` entries this supersedes. Verify with the same live DevTools Protocol
query that found the regression and that caught the prior attempt's failure, using the
`verifying-begin-ui` skill — `cargo build`/`clippy` cannot detect this class of bug.

**Tech Stack:** Rust, Dioxus 0.7 (`document::Script`), Spectrum Web Components 1.12.2
(live esm.sh references), `verifying-begin-ui` skill (headless Edge + raw DevTools
Protocol).

## Global Constraints

- Scope is light color / medium scale only — matches `app.rs`'s only `SpTheme` usage
  today (`color: "light"`, `scale: "medium"`). Do not add dark theme or other scale
  variants.
- `begin` now has a runtime network dependency on esm.sh for Spectrum to render at
  all — this is an intentional, accepted trade-off (confirmed with the human) in
  exchange for correct module sharing without new tooling. Do not try to route these
  three specific script tags back through `versions.toml`/`cargo xtask fetch-assets` —
  that vendoring mechanism is for self-contained single-file assets, which these
  cannot be if module identity must be shared across files (see the spec's Root Cause
  section for why). D3 and the zoom icons are unaffected and stay vendored exactly as
  they are.
- This fix cannot be verified by `cargo build`/`cargo clippy` alone — it was invisible
  to both, twice (the original regression and the first, superseded fix attempt). The
  proof is a live DevTools Protocol query showing `sp-theme`'s shadow root has adopted
  stylesheets and a real computed font-family, per
  `.claude/skills/verifying-begin-ui/SKILL.md`.
- Before opening a PR: `cargo fmt --all`, a zero-warning `cargo build --workspace` and
  `cargo test --workspace`, and all three clippy invocations (`cargo clippy
  --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin
  --no-default-features --all-targets -- -D warnings`, `cargo clippy -p begin
  --all-targets -- -D warnings`) must pass, per `CLAUDE.md`.

---

### Task 1: Load Spectrum live from esm.sh and verify the fix

**Files:**
- Modify: `begin/src/app.rs`

**Interfaces:** none new — this task only changes which URLs three `document::Script`
tags point at.

- [ ] **Step 1: Replace the `swc.js` script tag with three live esm.sh tags**

In `begin/src/app.rs`, replace:

```rust
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-in.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-out.js") }
```

with:

```rust
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        // Loaded live from esm.sh rather than vendored: Spectrum 2's theme-fragment
        // registration is module-scoped class state, not a global browser registry
        // like customElements.define. A self-contained per-file bundle inlines its own
        // private Theme class copy, so registration never reaches the real <sp-theme>
        // instance (verified live - see docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md).
        // esm.sh's *plain* (non-.bundle.mjs) modules instead resolve their shared
        // Theme.mjs dependency to the same absolute URL, so the browser's own ES module
        // cache shares one real instance across all three.
        document::Script { r#type: "module", src: "https://esm.sh/@spectrum-web-components/bundle@1.12.2/elements.js" }
        document::Script { r#type: "module", src: "https://esm.sh/@spectrum-web-components/theme@1.12.2/spectrum-two/theme-light.js" }
        document::Script { r#type: "module", src: "https://esm.sh/@spectrum-web-components/theme@1.12.2/spectrum-two/scale-medium.js" }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-in.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-out.js") }
```

- [ ] **Step 2: Build**

Run: `cargo build -p begin --no-default-features`
Expected: builds with no errors or warnings. (`begin/assets/swc.js` is now unreferenced
by any `asset!()` call — that's expected and fine; Task 2 removes the now-dead file.)

- [ ] **Step 3: Lint (both begin variants)**

Run:
```bash
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: no warnings from either invocation.

- [ ] **Step 4: Live verification via the `verifying-begin-ui` skill**

Read `.claude/skills/verifying-begin-ui/SKILL.md` and follow it. From `begin/`:

```bash
dx serve --platform web --no-default-features --features web --open false > /tmp/dx.log 2>&1 &
disown
until curl -sf http://localhost:8080 >/dev/null; do sleep 1; done
```

Launch Edge with remote debugging and find the page's target id:

```bash
"/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
  --headless=new --disable-gpu --no-sandbox --remote-debugging-port=9222 \
  --window-size=1280,900 http://localhost:8080 &
disown
sleep 3
curl -s http://127.0.0.1:9222/json | grep -B3 'localhost:8080'   # copy the "id" value
```

Run the fix's actual proof query (pass the bare `id` from above, NOT the full
`/devtools/page/<id>` path - see the skill's git-bash gotcha):

```bash
python3 .claude/skills/verifying-begin-ui/cdp_eval.py <id> "JSON.stringify({
  themeAdoptedSheets: (function(){var t=document.querySelector('sp-theme'); return t && t.shadowRoot ? t.shadowRoot.adoptedStyleSheets.length : 'no-shadow';})(),
  actionButtonFont: (function(){var b=document.querySelector('sp-action-button'); return b ? getComputedStyle(b).fontFamily : 'no-element';})()
})"
```

**This step's pass condition:** `themeAdoptedSheets` must be `> 0` (it has been `0`
both before any fix and after the first, superseded fix attempt) and
`actionButtonFont` must NOT be `"Times New Roman"`. If either check fails, this task
is not done - do not proceed to Step 5 or claim success. Given this is the *second*
attempt at this exact fix, a failure here means the live-esm.sh approach itself needs
re-diagnosis (e.g. an esm.sh URL 404ing, or a CSP/mixed-content block) - do not guess a
third variant without fresh evidence; report BLOCKED with the raw CDP response.

Also take a screenshot for a visual sanity check:

```bash
"/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
  --headless=new --disable-gpu --no-sandbox --window-size=1280,900 \
  --virtual-time-budget=8000 --screenshot=/tmp/theme-fix-shot.png http://localhost:8080
```

Read `/tmp/theme-fix-shot.png` with the Read tool and confirm it visibly shows
Spectrum styling (real typography, colors, and rounded corners on the zoom control),
not the plain-HTML rendering from the original bug report.

Clean up when done: `taskkill //F //IM msedge.exe` and `taskkill //F //IM dx.exe`.

- [ ] **Step 5: Commit**

```bash
git add begin/src/app.rs
git commit -m "fix(begin): load Spectrum live from esm.sh for correct theme-token sharing"
```

---

### Task 2: Remove the now-dead vendored assets

**Files:**
- Modify: `begin/assets/versions.toml`
- Delete: `begin/assets/swc.js`, `begin/assets/swc-theme-light.js`, `begin/assets/swc-scale-medium.js`

**Interfaces:** none — this task removes files/config nothing in the codebase
references anymore after Task 1.

- [ ] **Step 1: Edit `begin/assets/versions.toml`**

Remove the `[spectrum-web-components]`, `[spectrum-theme-light]`, and
`[spectrum-scale-medium]` sections, leaving:

```toml
[d3]
version = "7.9.0"
url = "https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js"
file = "d3.v7.min.js"

[spectrum-icon-zoom-in]
# icons-workflow is not part of the main bundle package (verified: bundle's elements.js
# entry point has no "icons-workflow" import), so each icon needed is vendored separately.
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/icons-workflow@1.12.2/es2022/icons/sp-icon-zoom-in.bundle.mjs"
file = "swc-icon-zoom-in.js"

[spectrum-icon-zoom-out]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/icons-workflow@1.12.2/es2022/icons/sp-icon-zoom-out.bundle.mjs"
file = "swc-icon-zoom-out.js"
```

- [ ] **Step 2: Delete the now-dead vendored files**

```bash
git rm begin/assets/swc.js begin/assets/swc-theme-light.js begin/assets/swc-scale-medium.js
```

- [ ] **Step 3: Confirm the remaining vendored assets still fetch correctly**

Run: `cargo xtask fetch-assets`
Expected: three `Fetching ... v...` / `-> ... (... bytes)` pairs (d3,
spectrum-icon-zoom-in, spectrum-icon-zoom-out), no `error:` line, and no attempt to
fetch the removed entries.

- [ ] **Step 4: Build and lint**

Run:
```bash
cargo build -p begin --no-default-features
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: all three succeed with zero errors/warnings, confirming nothing else in the
codebase referenced the removed files.

- [ ] **Step 5: Commit**

```bash
git add begin/assets/versions.toml
git commit -m "chore(begin): remove superseded self-contained Spectrum bundle assets"
```

---

### Task 3: Full workspace verification before PR

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (if any, stage exactly those files - don't use `git add -A`, there
may be unrelated pre-existing uncommitted changes in the working tree that must not be
swept in).

- [ ] **Step 2: Full workspace build**

Run: `cargo build --workspace`
Expected: zero warnings, zero errors.

- [ ] **Step 3: Full workspace tests**

Run:
```bash
cargo test --workspace
cargo test --doc --workspace
```
Expected: all pass, zero warnings in the build output.

- [ ] **Step 4: All three clippy invocations**

Run:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: no warnings from any of the three.

## Self-Review Notes

- **Spec coverage:** "`app.rs` changes" (live script tags) → Task 1. "Verification"
  (live DevTools query + screenshot) → Task 1/Step 4. "`versions.toml` /
  vendored-file cleanup" → Task 2. Repo-wide PR checklist → Task 3. No spec section
  is uncovered.
- **Placeholder scan:** no TBD/TODO markers; every step has complete, literal code,
  exact commands, and explicit pass/fail conditions.
- **Type consistency:** N/A - this plan only changes script-tag URLs and removes
  files/config, no new Rust types/functions/signatures.
- **Ordering check:** Task 1 (wire live references) intentionally runs before Task 2
  (delete the now-dead vendored files) so `begin` never has a broken intermediate
  state where `app.rs` references an asset that doesn't exist on disk.
