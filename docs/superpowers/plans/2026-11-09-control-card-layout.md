# Control Sizing and Card Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the bound controls at the dioxus-components example sizes in every host, put
sliders to the left of their number fields, and present the book and slides inspectors as a
centered Card without the "Cells" header.

**Architecture:** Follow the dioxus-components convention throughout: unstyled
`dioxus-primitives` behavior plus a styled component whose `component.rs` pairs with a
component-local `style.css` CSS module (classes via `Styles::…`). The `adam-web-ui` build script
compiles every module; it also rewrites each `<n>rem` length to
`calc(<n> * var(--adam-rem, 1rem))`, so a host whose root font size is not 16px (mdBook's
62.5%) restores upstream proportions with one custom property. The upstream `card` is vendored;
`NumericEditor`/`NumericSlider` and `SheetInspector` move their styles from the shared theme
stylesheet and inline `style:` attributes into their own modules. `SheetInspector` gains a
presentation choice: Begin keeps the header panel; the book and slides select the card.

**Tech Stack:** Rust 2024, Dioxus 0.7, `dioxus-primitives` at
`9a758255ea26e2b20c8cecf4c1c946feb9e71da7`, Lightning CSS build-time transform, Playwright.

**Spec:** Approved in conversation (2026-11-09):

1. Match the dioxus-components examples: labels 12.8px, native input text (13.33px, 31px tall),
   8px between label and field, 24px between fields, cards up to 24rem (384px) wide.
2. A ranged numeric cell shows its label above one row: slider on the left, filling the
   remaining width, and a fixed-width number field on the right.
3. Numeric help text ("Enter a number. Arrow Up…") becomes visually hidden and remains the
   input's accessible description.
4. Vendor the upstream Card with provenance. The book and slides render the inspector inside
   `Card`/`CardContent`, without the "Cells" header, at most 24rem wide and horizontally
   centered. Begin keeps its header and 260px side panel.

## Global Constraints

- Work only in `.claude/worktrees/issue-272` on branch `worktree-issue-272`; leave changes
  uncommitted unless the user asks; never stage `.scout/memory/*` or the screenshot
  `/Users/sean-parent/repos/github.com/stlab/cel-rs.worktrees/image.png`.
- Vendored files keep upstream content; local adaptations are documented in
  `adam-web-ui/src/components/UPSTREAM.md`.
- Every styled component (vendored or local) owns a `style.css` CSS module compiled by
  `adam-web-ui/build.rs` with naming pattern `adam_<module>_[local]`; the theme stylesheet holds
  only tokens and the `.adam-theme` scope. No inline `style:` attributes for component layout.
- Every Rust item gets a contract-style `///` comment; public APIs get `# Examples`.
- Grammar, model behavior, and nudge semantics are unchanged.

## Review Focus

- Begin's 236px-wide content column must fit a slider row: the slider's upstream
  `min-width: 200px` would overflow; the browser test asserts no horizontal overflow.
- An empty `role="status"` element must not add vertical space, but must stay in the
  accessibility tree so later errors are announced.
- Lengths inside `calc()` and `transform` values (switch thumb) must also be converted;
  the build-output test scans the whole embedded stylesheet.
- Slides already test that inspector content fits without scrollbars; card padding must not
  regress those tests.
- Unranged numeric cells and checkboxes/text fields inside the card keep the 24px spacing.

---

### Task 1: Host-scalable control lengths

**Files:**
- Modify: `adam-web-ui/build.rs`
- Modify: `adam-web-ui/assets/dx-components-theme.css` (document `--adam-rem` on `.adam-theme`)
- Modify: `adam-web-ui/src/components/UPSTREAM.md`
- Modify: `adam-lang-book/book-src/theme/inspector.css` (`.adam-theme { --adam-rem: 1.6rem; }`)
- Test: `adam-web-ui/src/components/tests.rs`

**Interfaces:**
- Produces: every embedded length `<n>rem` is emitted as `calc(<n> * var(--adam-rem, 1rem))`;
  hosts scale controls by setting `--adam-rem` on or above `.adam-theme`.

- [ ] **Step 1: Write the failing test** `embedded_lengths_scale_with_adam_rem`: no ASCII digit
  or `.` in `CONTROL_CSS` is immediately followed by `rem` except inside the fallback
  `var(--adam-rem, 1rem)`; `CONTROL_CSS` contains `var(--adam-rem, 1rem)`.
- [ ] **Step 2:** `cargo test -p adam-web-ui --lib embedded_lengths_scale` → FAIL.
- [ ] **Step 3: Implement** `fn scale_rem(source: &str) -> String` in `build.rs`: a single
  forward scan replacing each numeric literal (not preceded by an identifier character or `-`
  that belongs to an identifier) immediately followed by `rem` and a non-identifier character
  with the `calc(...)` form; applied to each source before `StyleSheet::parse`. Contract
  comment with O(b) complexity.
- [ ] **Step 4:** Re-run → PASS; `cargo test -p adam-web-ui` → all pass.
- [ ] **Step 5:** Book CSS sets `--adam-rem: 1.6rem` on `.adam-theme`; slides set it in
  `adam-slides/example.css` only if the slides root font size is not 16px (Task 3's browser
  test asserting 12.8px labels decides). UPSTREAM.md documents the rewrite and the property.

### Task 2: Vendor the Card

**Files:**
- Create: `adam-web-ui/src/components/card/{mod.rs,component.rs,style.css}` from upstream
  `preview/src/components/card/` (all seven components)
- Modify: `adam-web-ui/build.rs` (add `"card"` to the module list)
- Modify: `adam-web-ui/src/components/mod.rs` (`pub mod card;` and
  `pub use card::{Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle};`)
- Modify: `adam-web-ui/src/components/UPSTREAM.md` (directory list and `dx components add` command)

**Interfaces:**
- Produces: `Card`, `CardContent` (each accepts `GlobalAttributes` and `children`), rendering
  `div[data-slot="card"]` / `div[data-slot="card-content"]` and calling `use_control_styles()`.

- [ ] **Step 1: Write the failing SSR test** `card_renders_upstream_slots` in
  `components/tests.rs`: render `Card { style: "max-width: 24rem", CardContent { "x" } }`, assert
  `data-slot="card"`, `data-slot="card-content"`, `adam_card_dx-card`, and the style attribute.
- [ ] **Step 2:** Run → FAIL (unresolved `Card`).
- [ ] **Step 3:** Vendor following the `label` adaptation pattern: replace `#[css_module]` with
  `include!(concat!(env!("OUT_DIR"), "/card-styles.rs"))`, add `use_control_styles()`, contracts,
  module tutorial with `# Examples`.
- [ ] **Step 4:** Run → PASS.

### Task 3: Numeric editor as a CSS-module component; spacing, hidden help, slider row

**Files:**
- Move: `adam-web-ui/src/components/numeric_editor.rs` → `numeric_editor/component.rs`;
  `numeric_editor_tests.rs` → `numeric_editor/tests.rs`; new `numeric_editor/mod.rs`
  (module tutorial, `pub use component::*;`) and `numeric_editor/style.css`
- Modify: `adam-web-ui/build.rs` (add `"numeric_editor"`), `components/mod.rs`
- Modify: `adam-web-ui/assets/dx-components-theme.css` (delete all `[class~="adam-numeric-…"]` rules)
- Test: `adam-web-ui/src/components/tests.rs:1069`, `adam-slides/tests/slides.spec.mjs`

**Interfaces:**
- Produces: `Styles::{editor, controls, stepper, slider, visually_hidden}` from
  `numeric_editor/style.css` (scoped `adam_numeric_editor_<local>`). Public API unchanged.

Layout (all in the module): `.editor` is a grid with `gap: 0`; its label has
`margin-bottom: 0.5rem`; the `role="status"` element has no minimum height (stays in the DOM).
`.controls`/`.stepper` carry the current frame and nudge rules unchanged. `.slider` is a grid
(`grid-template-columns: minmax(0, 1fr) 7rem; column-gap: 0.75rem; align-items: center`); inside
it `.editor` is `display: contents`, the label spans row 1, the slider is row 2 column 1 with
`min-width: 0`, `.controls` is row 2 column 2, and the status spans row 3. `.visually_hidden` is
the clip-based screen-reader-only pattern applied to the help `small`.

- [ ] **Step 1: Extend the SSR test** at `tests.rs:1069`: the help element's class contains
  `adam_numeric_editor_visually_hidden` and it remains referenced by `aria-describedby`.
- [ ] **Step 2: Write failing browser tests** in `slides.spec.mjs`:
  - `ranged slider sits left of its number field`: the slider's box ends before the number
    input begins, vertical centers differ by < 4px, and the label's bottom is above both.
  - `controls match dioxus-components sizes`: label computed `font-size` is `12.8px`, input
    height 31px, label-to-field distance 8px (±1), help box at most 1×1, and the inspector has
    no horizontal overflow.
- [ ] **Step 3:** Run them → FAIL.
- [ ] **Step 4:** Move files, create the module, switch markup to `Styles::…`, delete theme rules.
- [ ] **Step 5:** Rebuild WASM/assets; run the new tests and the existing numeric tests (frame,
  disabled nudges, integrated nudges, slider drag) → PASS; `cargo test -p adam-web-ui` → PASS.

### Task 4: Card presentation for the book and slides

**Files:**
- Move: `adam-web-ui/src/inspector.rs` → `inspector/mod.rs`; `inspector_tests.rs` →
  `inspector/tests.rs`; new `inspector/style.css`
- Modify: `adam-web-ui/build.rs` (accept `(module, path)` entries so `"inspector"` maps to
  `src/inspector/style.css`)
- Modify: `adam-web-ui/src/lib.rs` (re-export `InspectorPresentation`)
- Modify: `xtask/src/live_book_assets.rs:257,299` if its fixtures assume the old selectors
- Modify: `adam-lang-book-live/src/lib.rs:447`
- Modify: `adam-lang-book/book-src/theme/inspector.css` and `adam-slides/example.css`
- Test: `adam-web-ui/src/inspector_tests.rs`, `adam-slides/tests/slides.spec.mjs`

**Interfaces:**
- Produces: `pub enum InspectorPresentation { #[default] Panel, Card }` (`Clone, Copy, PartialEq, Debug, Default`);
  `SheetInspector(sheet: SheetHandle, #[props(default)] presentation: InspectorPresentation)`.
  Module classes `Styles::{panel, card, cells}` from `inspector/style.css`. `Panel` keeps the
  current 260px panel (moved from the inline `style:` into `.panel`), `h2 { "Cells" }`, and
  `Separator`. `Card` renders `Card { class: "{Styles::card} adam-sheet-inspector", CardContent { rows } }`
  with `.card { width: 100%; max-width: 24rem; margin-inline: auto; box-sizing: border-box; }`.
  In both, rows sit in `div.{Styles::cells}` (flex column, `gap: 1.5rem`). The unscoped
  `adam-sheet-inspector` class stays as the public host hook, like `adam-theme`.

- [ ] **Step 1: Write failing SSR tests** `card_presentation_omits_header` (contains
  `data-slot="card"`, no `<h2>`, contains every cell label) and
  `panel_presentation_keeps_header` (default contains `Cells` and no `data-slot="card"`).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement; the book host passes `presentation: InspectorPresentation::Card`;
  delete the `.adam-cell-row` margin rule from the book CSS.
- [ ] **Step 4: Write a failing browser test** `inspector is a centered card`: `[data-slot=card]`
  exists, no `h2` in `#inspector`, card width ≤ 384px, and its horizontal center is within
  2px of `#inspector`'s content center; consecutive `.adam-cell-row` tops differ by row
  height + 24px (±1).
- [ ] **Step 5:** Rebuild and run the full slides suite → all pass.

### Task 5: Whole-change verification

- [ ] `cargo fmt --all -- --check`; `cargo test -p adam-web-ui`; `cargo test -p adam-lang-book-live`;
  `cargo test -p begin` (still expects "Cells").
- [ ] Clippy: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` and
  both `begin` invocations; `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps -p adam-web-ui`.
- [ ] Render the book with `mdbook build . -d <scratch>` and inspect the tutorial page: centered
  card, no header, 12.8px labels, slider left of field.
- [ ] Render Begin with the `verifying-begin-ui` skill: header kept, slider rows fit 260px.
- [ ] Delete screenshots and scratch builds.
