# Begin: Fix Spectrum 2 Theme-Token Loading Regression Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Spectrum 2 migration regression where no Spectrum design-token CSS
applies anywhere in `begin`'s UI, by vendoring and loading the two theme-token
modules `<sp-theme system="spectrum-two">` needs but never received.

**Architecture:** Add two new vendored, self-contained esm.sh asset entries to
`versions.toml` (the actual `--spectrum-*` token CSS for light color / medium scale
under the `spectrum-two` system), fetch them via the existing `cargo xtask
fetch-assets` mechanism, and load them as two new `<script type="module">` tags in
`app.rs` alongside the existing `swc.js` tag. Verify with a live DevTools Protocol
query (not just `cargo build`/`clippy`, which cannot detect this class of bug) using
the `verifying-begin-ui` skill.

**Tech Stack:** Rust, Dioxus 0.7 (`document::Script`), Spectrum Web Components 1.12.2
(esm.sh-vendored), `xtask` asset fetcher, `verifying-begin-ui` skill (headless Edge +
raw DevTools Protocol).

## Global Constraints

- Scope is light color / medium scale only — matches `app.rs`'s only `SpTheme` usage
  today (`color: "light"`, `scale: "medium"`). Do not vendor dark theme or other scale
  variants.
- Every vendored asset must be a self-contained single-file ES module (no external
  runtime network dependency) — use the exact `es2022/spectrum-two/*.bundle.mjs`
  esm.sh URLs verified during brainstorming, not bare (non-bundled) entry points.
- `versions.toml` is the single source of truth for vendored asset versions/URLs;
  assets are updated via `cargo xtask fetch-assets`, never hand-edited or
  hand-downloaded.
- This fix cannot be verified by `cargo build`/`cargo clippy` alone — it was invisible
  to both the first time. The proof is a live DevTools Protocol query showing
  `sp-theme`'s shadow root has adopted stylesheets and a real computed font-family,
  per `.claude/skills/verifying-begin-ui/SKILL.md`.
- Before opening a PR: `cargo fmt --all`, a zero-warning `cargo build --workspace` and
  `cargo test --workspace`, and all three clippy invocations (`cargo clippy
  --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin
  --no-default-features --all-targets -- -D warnings`, `cargo clippy -p begin
  --all-targets -- -D warnings`) must pass, per `CLAUDE.md`.

---

### Task 1: Vendor the Spectrum 2 theme-token assets

**Files:**
- Modify: `begin/assets/versions.toml`
- Generated (via `cargo xtask fetch-assets`, commit the output):
  `begin/assets/swc-theme-light.js`, `begin/assets/swc-scale-medium.js`

**Interfaces:**
- Produces: `begin/assets/swc-theme-light.js` and `begin/assets/swc-scale-medium.js`
  (new files that Task 2 wires into `app.rs` as additional script tags).

- [ ] **Step 1: Edit `begin/assets/versions.toml`**

Append two new sections after the existing `[spectrum-icon-zoom-out]` section:

```toml
[spectrum-theme-light]
# The main "bundle" package's elements.bundle.mjs has sp-theme's *class logic* but not
# the actual --spectrum-* token *values* for a given system/color - those live in this
# separate module, meant to be loaded as its own <script type="module"> tag in
# non-bundler setups (confirmed via the theme package's own exports map).
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/theme@1.12.2/es2022/spectrum-two/theme-light.bundle.mjs"
file = "swc-theme-light.js"

[spectrum-scale-medium]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/theme@1.12.2/es2022/spectrum-two/scale-medium.bundle.mjs"
file = "swc-scale-medium.js"
```

- [ ] **Step 2: Fetch the assets**

Run: `cargo xtask fetch-assets`

Expected output: one `Fetching <name> v1.12.2 ...` / `-> <path> (<bytes>)` pair for
each of the six `versions.toml` entries (d3, spectrum-web-components,
spectrum-icon-zoom-in, spectrum-icon-zoom-out, spectrum-theme-light,
spectrum-scale-medium), with no `error:` line. `swc-theme-light.js` should be roughly
400-600KB and `swc-scale-medium.js` roughly 150-300KB (both are large, token-heavy
files - a file under a few KB means the wrong URL was fetched).

- [ ] **Step 3: Verify the fetched files contain real token definitions**

Run (from the repo root):

```bash
grep -c -- "--spectrum-global-color" begin/assets/swc-theme-light.js
grep -c -- "--spectrum-alias-item-height" begin/assets/swc-scale-medium.js
grep -c "adoptedStyleSheets" begin/assets/swc-theme-light.js
```

Expected: each command prints a number `>= 1`. If any prints `0`, the fetch pulled the
wrong file - stop and re-check the URL in `versions.toml`.

- [ ] **Step 4: Commit**

```bash
git add begin/assets/versions.toml begin/assets/swc-theme-light.js begin/assets/swc-scale-medium.js
git commit -m "chore(begin): vendor Spectrum 2 theme-light/scale-medium token modules"
```

---

### Task 2: Load the theme-token scripts and verify the fix live

**Files:**
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: `begin/assets/swc-theme-light.js` and `begin/assets/swc-scale-medium.js`
  from Task 1.

- [ ] **Step 1: Add the two new script tags in `begin/src/app.rs`**

Find the existing script tags (added in the prior plan on this branch):

```rust
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-in.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-out.js") }
```

Add two more directly after them:

```rust
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-in.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-out.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-theme-light.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-scale-medium.js") }
```

- [ ] **Step 2: Build**

Run: `cargo build -p begin --no-default-features`
Expected: builds with no errors or warnings.

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

**This step's pass condition:** `themeAdoptedSheets` must be `> 0` (it was `0` before
this fix) and `actionButtonFont` must NOT be `"Times New Roman"` (it must reference a
real font stack). If either check fails, this task is not done - do not proceed to
Step 5 or claim success; re-check `versions.toml`'s URLs and `app.rs`'s script tags
against Task 1/Step 1.

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
git commit -m "fix(begin): load spectrum-two theme-light/scale-medium tokens"
```

---

### Task 3: Full workspace verification before PR

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (if any, stage exactly those files and fold into a follow-up
`chore(begin): cargo fmt` commit - don't use `git add -A`, there may be unrelated
pre-existing uncommitted changes in the working tree that must not be swept in).

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

- **Spec coverage:** "New vendored assets" → Task 1. "`app.rs` changes" → Task 2/Step
  1. "Verification" (live DevTools query + screenshot) → Task 2/Step 4. Repo-wide PR
  checklist → Task 3. No spec section is uncovered.
- **Placeholder scan:** no TBD/TODO markers; every step has complete, literal code,
  exact commands, and explicit pass/fail conditions (not just "verify it works").
- **Type consistency:** N/A - this plan only adds asset files and script tags, no new
  Rust types/functions/signatures are introduced.
