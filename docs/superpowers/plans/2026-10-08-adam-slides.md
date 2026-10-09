# Adam Slides Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and publish a static Marp presentation with nine live Adam tutorial examples.

**Architecture:** Generate slide source from the book's canonical Adam files and render it with Marp CLI. Each slide embeds a same-site static example host that mounts the existing WebAssembly inspector and optional graph. Reuse the book's asset staging and publish the standalone directory through the existing Pages workflow.

**Tech Stack:** Rust 2024 xtask, Node.js 22 or newer, Marp CLI 4.5.1, existing Dioxus/WebAssembly UI, Node's built-in test runner, Playwright Test 1.64.0.

**Spec:** `docs\superpowers\specs\2026-10-08-adam-slides-design.md`

## Global Constraints

- The deliverable is a self-contained static website suitable for GitHub Pages.
- There is no application server, runtime CDN dependency, or dependency on the published book.
- The initial deck has exactly nine example slides, in the tutorial's order.
- Source code is displayed, not edited.
- Do not commit duplicate Adam sheets.
- Keep slide titles and prompts editable in the deck's Markdown.
- Preserve the existing book command's destination and behavior.
- Keep each frame mounted after it first loads so navigating away and back preserves edits.
- Missing examples and asset/runtime failures must show visible failures, not blank frames.
- Generated files are not committed.
- Preserve the existing books and rustdoc; do not add a second Pages deployment workflow.
- Use Windows paths in local commands; existing Linux CI uses its native path syntax.
- Add adjacent contracts to new functions and types, including JavaScript helpers and tests.
- Before every commit, run `cargo fmt --all`; stage only task-owned changes.
- Every commit includes `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.

## Review Focus

- A Pages project served beneath multiple path segments resolves every frame, script, manifest, and WebAssembly asset without origin-root assumptions: Task 3.
- Commented tutorial includes or duplicate deck references cannot add or hide example slides: Task 2.
- Spectrum controls are actually upgraded and user events propagate, rather than merely displaying plausible HTML: Task 3.
- First visiting a hidden graph slide and returning after a resize produces a nonzero, usable graph without resetting the sheet: Task 3.
- Missing assets, invalid selectors, and failed HTTP responses produce visible failures; previous cached output cannot disguise a failed build: Tasks 1-3.

---

## File Structure and Dependencies

All new paths below are proposed. Existing modification sites were read through Scout.

| File | Responsibility |
| --- | --- |
| `xtask\src\live_book_assets.rs` | Shared manifest and runtime asset staging; book and slides wrappers. |
| `xtask\src\main.rs` | Register the slides staging command. |
| `adam-slides\package.json`, `package-lock.json`, `.gitignore` | Reproducible build and test commands; exclude generated output and dependencies. |
| `adam-slides\slides.md`, `slides.css` | Nine authored slides, interaction prompts, presentation layout. |
| `adam-slides\deck.mjs`, `deck.test.mjs` | Resolve source directives and validate tutorial coverage. |
| `adam-slides\build.mjs`, `build.test.mjs` | Stage assets and host, generate Markdown, surface build failures. |
| `adam-slides\example.html`, `example.css` | Accessible loading/error surface and bounded inspector/graph layout. |
| `adam-slides\example.mjs`, `example.test.mjs` | Selector validation, dependency loading, and one-time mount. |
| `adam-slides\playwright.config.mjs`, `tests\slides.spec.mjs`, `tests\serve.mjs` | Nested-path static test server and real browser acceptance checks. |
| `adam-slides\README.md` | Clean-checkout build, preview, publication, and HTML-only interactivity. |
| `.github\workflows\docs.yml` | Build, verify, and include slides in the existing Pages artifact. |

Task 1 and Task 2 have independent test cycles. Task 3 consumes both; Task 4
consumes the complete build and browser tests.

## Task 1: Share Live Asset Staging

**Files:** Modify `xtask\src\live_book_assets.rs:97-157` and `xtask\src\main.rs:103-129`; add unit tests in the staging module.

**Interfaces:**
- Consumes: existing `build_manifest(&Path)` and `copy_dir_contents(&Path, &Path)`.
- Produces: `prepare_live_assets(root: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>>`.
- Retains: `prepare_live_book_assets() -> Result<(), Box<dyn std::error::Error>>`.
- Adds: `prepare_live_slides_assets() -> Result<(), Box<dyn std::error::Error>>`, staging to `adam-slides\dist\theme`.
- CLI: `cargo run -p xtask -- prepare-live-slides-assets`.

- [ ] **Step 1: Write failing contract tests.**
  Create uniquely named temporary fixtures with canonical example directories,
  `begin\assets`, and a fake `adam-lang-book-live\pkg` tree. Assert that staging
  writes the manifest, copies the five existing assets, and preserves nested
  `snippets` and binary WebAssembly bytes. Assert that `NO_LIVE_MOUNT` entries
  remain excluded. Assert missing source, vendored asset, or pkg returns `Err`,
  including when destination files already exist. Clean only the owned fixture.
- [ ] **Step 2: Run `cargo test -p xtask live_book_assets`.**
  Expected: failure because the shared entry point does not exist.
- [ ] **Step 3: Extract shared staging and add the slides wrapper.**
  Preserve the book's default destination and manifest behavior. Check required
  source assets and pkg existence before writing output, propagate I/O failures,
  and document the shared helper's errors and complexity.
- [ ] **Step 4: Register the new command and update usage text.**
  Follow the existing stderr reporting and nonzero exit pattern. Do not introduce
  arbitrary destination configuration or alter other command dispatch.
- [ ] **Step 5: Run `cargo test -p xtask` and `cargo clippy -p xtask --all-targets -- -D warnings`.**
  Expected: all staging contract tests pass; no warnings.
- [ ] **Step 6: Format and commit the staging changes.**
  Commit message: `Share live example asset staging with Adam slides`.

## Task 2: Generate the Nine Canonical Slides

**Files:** Create the package files, `.gitignore`, `slides.md`, `slides.css`, `deck.mjs`, and `deck.test.mjs`.

**Interfaces:**
- `tutorialExamples(markdown: string) -> Array<{ key: string, graph: boolean }>` reads active tutorial includes and graph declarations in order.
- `renderDeck(markdown: string, examples: Array<{ key: string, graph: boolean }>, readSource: (key: string) => string) -> string` validates coverage and expands authored directives.
- Authored directive: `<!-- adam-example: tutorial/first_sheet -->`, one per slide.
- Rendered frame: `example.html?example=tutorial%2Ffirst_sheet&graph=1`; non-graph slides use `graph=0`.
- Every frame has `title`, `data-example`, and eager loading; source occupies a fenced Adam code block.

- [ ] **Step 1: Create the package and failing generator tests.**
  Pin `@marp-team/marp-cli` to `4.5.1` and `@playwright/test` to `1.64.0`
  as development dependencies; set `engines.node` to `>=22`.
  Use `npm install` only after writing the manifest, and commit its generated
  lockfile. Ignore `node_modules`, `dist`, `test-results`, and `playwright-report`.
  Set `npm test` to `node --test *.test.mjs`.
  Tests assert the exact ordered keys:
  `first_sheet`, `clamp_demo`, `basic_output`, `basic_relationship`, `inequality`,
  `constrain`, `conditional_forced`, `requirements_filter_diagnostic`,
  `area_with_requirement`, each prefixed with `tutorial/`.
  Assert graph flags are `[true, false, true, true, true, true, false, false, false]`.
  Assert commented-out includes are ignored; duplicate, missing, reordered,
  unknown, malformed, and traversal-like deck references fail explicitly.
  Assert canonical source survives rendering, including HTML-like text and
  Markdown backticks, without becoming executable markup or breaking its fence.
- [ ] **Step 2: Run `npm test` from `adam-slides`.**
  Expected: generator tests fail because their imported module is missing.
- [ ] **Step 3: Implement the two generator interfaces in `deck.mjs`.**
  Strip HTML comments when collecting active tutorial examples; parse only
  the documented include and graph syntax. Validate keys against
  `^tutorial/[a-z][a-z0-9_]*$` and exact tutorial membership before reading files.
  Escape frame attributes and choose code fences longer than source backtick runs.
  Never interpolate Adam source into raw HTML.
- [ ] **Step 4: Author the nine slides and presentation theme.**
  Use the spec's exact titles and graph presence. Include one directive on each
  slide, a short prompt, widescreen sizing, and bounded source/live panes.
  Declare `/* @theme adam-slides */` in `slides.css` and select `theme: adam-slides`
  in the Markdown front matter.
  Keep full source available by scrolling; avoid external fonts or images.
- [ ] **Step 5: Run `npm test`.**
  Expected: all generator tests pass with nine canonical references and no
  commented-out examples.
- [ ] **Step 6: Format and commit the generator and deck.**
  Commit message: `Add canonical Adam tutorial Marp slides`.

## Task 3: Build and Verify Live Static Examples

**Files:** Create the build, static host, runtime, and browser-test files from the file structure table.

**Interfaces:**
- `parseExampleOptions(url: URL) -> { key: string, graph: boolean }` accepts exactly one valid example key and a `graph` value of `0` or `1`.
- `exampleAssetUrls(base: URL) -> { module: URL, manifest: URL, spectrum: URL, d3: URL, graph: URL }` resolves assets in `theme/` relative to `example.html`.
- `loadExample(options: { key: string, graph: boolean }, urls: ReturnType<typeof exampleAssetUrls>) -> Promise<void>` loads dependencies and mounts once.
- Host DOM: `#status` with live status semantics, `#inspector`, and optional `#graph`; an error changes `#status` into a visible alert.
- `prepareDeck(root: string) -> Promise<void>` in `build.mjs` creates `dist`, invokes Task 1's command, renders Task 2's deck, and stages host files.
- `npm run build`: `node build.mjs && marp dist/slides.md --html --theme-set slides.css --output dist/index.html`.
- `npm run test:browser`: `playwright test`.

- [ ] **Step 1: Write failing runtime and build tests.**
  Assert selector validation rejects missing, duplicate, traversal, and invalid
  graph parameters. Assert asset URLs retain `/project/docs/adam-slides/`.
  Simulate unknown manifest entries, manifest HTTP 404, module/script rejection,
  and WebAssembly initialization rejection; each must produce a visible error.
  Assert graph dependencies finish before `mount` and no graph loader runs when
  graph is false. Assert staging subprocess failures reject the build even when
  stale output exists; never run Marp following a failed preparation.
- [ ] **Step 2: Run `npm test`.**
  Expected: new tests fail due to missing runtime/build interfaces.
- [ ] **Step 3: Implement the host and runtime.**
  Import `example.mjs` as a module; use URLs based on the host's document URL.
  Check HTTP success before JSON parsing and check own manifest membership.
  Load Spectrum and the existing module, initialize WebAssembly, and call
  `mount("inspector", source, key, graph ? ["graph"] : [])`.
  Await optional D3/graph loading first. Preserve existing UI styles and diagnostics.
  Link `theme/inspector.css`, `theme/graph.css`, and `example.css` from the host.
  Keep this error boundary narrowly around startup; display and log failures.
  Mount each eager iframe only once; do not add slide-driven reloads.
  Reuse graph.js's existing ResizeObserver behavior rather than replacing it.
- [ ] **Step 4: Implement static build preparation.**
  Use native path joins for files, not URL path joins. Generate `dist/slides.md`,
  copy the host/runtime/theme files, and use Task 1 for `dist/theme`.
  Preserve subprocess stderr and nonzero exit status. Validate required outputs,
  including the manifest and WebAssembly module, before declaring build readiness.
  Add the exact build and browser scripts from this task's Interfaces.
- [ ] **Step 5: Run `npm test` and build the real static output.**
  From the repository root run `wasm-pack build --target web --release .\adam-lang-book-live`.
  From `adam-slides` run `npm run build`.
  Expected: tests pass; `dist/index.html` and all host/runtime assets exist.
- [ ] **Step 6: Write browser acceptance tests and their static test server.**
  Serve output beneath `/project/docs/adam-slides/`, with correct HTML, JS, JSON,
  and WebAssembly MIME types. Resolve requested files inside `dist`, reject
  traversal, and return real 404s. Make Playwright manage the server lifecycle.
  Verify all nine slides contain canonical source and upgraded Spectrum controls.
  Use normal pointer/keyboard events in Spectrum's open shadow roots, not direct
  mutations of the Adam sheet.
  Assert: `width=30` makes basic output `area=600`; relationship `a=40` makes
  `b=20`, and editing `b=15` makes `a=30` with changed graph direction.
  For inequality, set `a=100`, then `a=0`, and assert `b=20`, `c=30`.
  Toggle `constrain` to enforce `a=b` in the conditional relationship example.
  In the forced example, assert activation makes `a=42` non-editable and
  deactivation restores `a=5` and editability.
  Exercise clamping to `100`, visible width filter diagnostics, and the area
  requirement violation when `width=100` and `height=100`.
  Edit first-sheet width, navigate away/back, and assert the edit persists.
  Assert frame input and graph dragging do not advance slides; presentation
  controls outside frames still navigate. Visit hidden graph slides, resize,
  and assert usable nonzero SVG dimensions without resetting values.
  Inspect screenshots at 1280x720 and 960x540 for legible source, controls,
  diagnostics, and unclipped graphs.
  Fail on unexpected browser errors or off-origin runtime requests.
  Intercept a runtime asset with HTTP 404 and assert the visible startup alert.
- [ ] **Step 7: Run browser and existing tutorial tests.**
  After manifest changes, install the browser with `npx playwright install chromium`
  (CI uses `--with-deps`). Run `npm run test:browser` and
  `cargo test -p adam-lang-book --test tutorial`.
  Expected: all interaction checks pass and the existing examples remain valid.
- [ ] **Step 8: Format and commit the live deck build.**
  Commit message: `Build and verify live static Adam presentation`.

## Task 4: Publish Through the Existing Pages Build

**Files:** Modify `.github\workflows\docs.yml`; create `adam-slides\README.md` and `docs\superpowers\2026-10-08-adam-slides-handoff.md`.

**Interfaces:**
- Consumes: `npm ci`, `npm test`, `npm run build`, and `npm run test:browser` in `adam-slides`.
- Produces: the existing Pages artifact with an `adam-slides/` static subtree.

- [ ] **Step 1: Verify a clean dependency restore and build.**
  Run `npm ci`, `npm test`, and `npm run build` in `adam-slides`.
  Expected: lockfile restore succeeds and the build does not depend on book-dist.
- [ ] **Step 2: Add slides build and browser verification to the Docs build job.**
  Set up Node 22 using the repository's current action/pinning conventions.
  Run npm commands with `working-directory: adam-slides` after the existing
  WebAssembly build. Install Chromium and run browser checks before upload.
  Copy `adam-slides/dist/.` into `target/doc/adam-slides/` with no extra nested
  `dist` directory. Preserve all existing doc builds, deployment gates, and permissions.
- [ ] **Step 3: Document reproducible build, local preview, and deployment.**
  Include Node 22, wasm-pack, wasm32 target, and Rust prerequisites; exact Windows
  commands for wasm-pack and npm; browser installation; and optional HTTP preview.
  State that the generated directory can be hosted at any nested static path and
  that only HTML retains interactivity.
- [ ] **Step 4: Verify the artifact-shaped directory.**
  Serve a temporary staging directory with the slides nested under the intended
  Pages path and reuse the browser suite. Expected: all static URLs resolve and
  no book, rustdoc, or Internet service is required by the slides.
- [ ] **Step 5: Run final focused checks and review.**
  Run `npm test`, `npm run test:browser`, `cargo test -p xtask`,
  `cargo test -p adam-lang-book --test tutorial`, and
  `cargo clippy -p xtask --all-targets -- -D warnings`.
  Review the branch using the selected execution method's review workflow.
  Fix in-scope findings before claiming completion. Do not open a PR without
  separately running the repository's full required pre-PR suite.
- [ ] **Step 6: Write the handoff, format, and commit publishing changes.**
  Record delivered behavior, verification, and any explicit blockers or deferred
  issues. Commit message: `Publish live Adam slides with the documentation site`.
