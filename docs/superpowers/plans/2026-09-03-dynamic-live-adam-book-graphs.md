# Dynamic Live adam-lang-book Graphs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make each book `<graph sheet="name">` update live from its example's inspector widgets, by having the inspector mount own the sheet and drive the associated graph container(s) imperatively.

**Architecture:** Restore `begin`'s single-`Signal<Sheet>` model across the book's DOM gap. The inspector `VirtualDom` for an example owns the sheet; a `use_effect` recomputes `to_graph_data` on every propagate and drives each associated graph container by id through the existing `window.beginGraph` seam. The graph's own frozen sheet (and `mount_graph`) is eliminated; the graph stays display-only (drag/zoom), with all edits flowing through the inspector widgets.

**Tech Stack:** Rust, Dioxus 0.7 (`web` feature), `wasm-bindgen`, D3 (via `begin/assets/graph.js`), an mdBook preprocessor (`regex`), and a plain-JS bootstrap script.

**Spec:** `docs/superpowers/specs/2026-09-03-dynamic-live-adam-book-graphs-design.md`

## Global Constraints

- Never commit to `main`; this work happens on the `worktree-adam-lang-book/add-graphs` branch.
- `cargo fmt --all` before every commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- Clippy warnings are errors, checked with three invocations before a PR:
  - `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
  - `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
  - `cargo clippy -p begin --all-targets -- -D warnings`
- Every function gets a `///` contract-style doc comment (Summary; plus Preconditions / `# Errors` / Postconditions / Complexity only where they apply). Derive unit tests from the contract and public interface, never from the implementation.
- The graph is display-only: no JS→wasm write-back, no editing from graph nodes.
- No structural edits (no adding/removing cells or relationships) from a live page.
- `begin` behavior must not change; verify with the `verifying-begin-ui` skill, since a passing build proves nothing about what renders.

---

### Task 1: Extract `graph_drive_script` and refactor `GraphView` to use it

**Files:**
- Modify: `adam-web-ui/src/graph/view.rs` (add helper + tests; call it from the `use_effect`)
- Modify: `adam-web-ui/src/graph/mod.rs:7` (export the helper)
- Modify: `adam-web-ui/src/lib.rs:15` (re-export the helper)

**Interfaces:**
- Consumes: `super::data::GraphData` (already imported in `view.rs`).
- Produces: `pub fn graph_drive_script(container_id: &str, data: &GraphData, call: &str) -> String` — builds the JS that stores the serialized `data` in `window.__beginGraphData[container_id]` and invokes `window.beginGraph.<call>(container_id, data)` when the driver is loaded. `call` is `"init"` or `"update"`. Task 3 consumes this.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-web-ui/src/graph/view.rs`:

```rust
fn empty_graph_data() -> GraphData {
    GraphData {
        nodes: vec![],
        links: vec![],
        changed: vec![],
        forced: vec![],
        forced_relationships: vec![],
        arrows: false,
    }
}

#[test]
fn graph_drive_script_init_calls_begin_graph_init() {
    let script = graph_drive_script("g1", &empty_graph_data(), "init");
    assert!(script.contains("window.beginGraph.init('g1'"));
    assert!(script.contains("window.__beginGraphData['g1']"));
}

#[test]
fn graph_drive_script_update_calls_begin_graph_update() {
    let script = graph_drive_script("g1", &empty_graph_data(), "update");
    assert!(script.contains("window.beginGraph.update('g1'"));
}

#[test]
fn graph_drive_script_guards_on_begin_graph_being_defined() {
    let script = graph_drive_script("g1", &empty_graph_data(), "init");
    assert!(script.contains("typeof window.beginGraph !== 'undefined'"));
}

#[test]
fn graph_drive_script_embeds_the_serialized_data() {
    let script = graph_drive_script("g1", &empty_graph_data(), "init");
    // GraphData serializes its fields; `nodes` is always present.
    assert!(script.contains("\"nodes\""));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-web-ui graph_drive_script`
Expected: FAIL to compile — `graph_drive_script` is not defined.

- [ ] **Step 3: Implement the helper**

Add to `adam-web-ui/src/graph/view.rs` (above `GraphView`):

```rust
/// Builds the JavaScript that stores `data` (serialized) as container `container_id`'s entry in
/// `window.__beginGraphData` and invokes `window.beginGraph.<call>(container_id, data)` when the
/// driver script (`begin/assets/graph.js`) is loaded.
///
/// - Precondition: `call` is `"init"` or `"update"`.
/// - Precondition: `container_id` contains no `'` (it is a DOM element id).
pub fn graph_drive_script(container_id: &str, data: &GraphData, call: &str) -> String {
    let json = serde_json::to_string(data).unwrap_or_default();
    format!(
        "window.__beginGraphData = window.__beginGraphData || {{}}; \
         window.__beginGraphData['{container_id}'] = {json}; \
         if (typeof window.beginGraph !== 'undefined') \
         window.beginGraph.{call}('{container_id}', window.__beginGraphData['{container_id}']);"
    )
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-web-ui graph_drive_script`
Expected: PASS (4 tests).

- [ ] **Step 5: Refactor `GraphView`'s effect to call the helper**

In `adam-web-ui/src/graph/view.rs`, replace the `use_effect` body's inline serialize + `format!` + `document::eval` (currently `view.rs:56-71`) with:

```rust
    use_effect(move || {
        let id = graph_id.read().clone();
        let current_source = source_id.read().clone();
        let is_new_source = source_changed(&current_source, &initialized_source.peek());
        if is_new_source {
            initialized_source.set(current_source);
        }
        let call = if is_new_source { "init" } else { "update" };
        let script = graph_drive_script(&id, &data.read(), call);
        spawn(async move {
            let _ = document::eval(&script).await;
        });
    });
```

(The `onmounted` polling-init handler at `view.rs:77-92` is left unchanged — it wraps init in a d3-load poll that is `begin`-specific and not part of this helper.)

- [ ] **Step 6: Export the helper**

In `adam-web-ui/src/graph/mod.rs`, change line 8 to:

```rust
pub use view::{GraphView, graph_drive_script};
```

In `adam-web-ui/src/lib.rs`, change the `graph` re-export (line 15) to add `graph_drive_script`:

```rust
pub use graph::{
    GraphData, GraphView, LinkData, LinkKind, NodeData, NodeKind, graph_drive_script,
    to_graph_data,
};
```

- [ ] **Step 7: Verify the whole crate builds and tests pass with no warnings**

Run: `cargo test -p adam-web-ui`
Expected: PASS, zero warnings.
Run: `cargo clippy -p adam-web-ui --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Confirm `begin` is unaffected**

Run: `cargo build -p begin`
Expected: builds (the `GraphView` signature is unchanged; only its effect internals moved).
Then use the `verifying-begin-ui` skill to render `begin` and confirm the graph still draws and zoom/fit/show-inactive still work. A passing build is not sufficient evidence.

- [ ] **Step 9: Commit**

```bash
git add adam-web-ui/src/graph/view.rs adam-web-ui/src/graph/mod.rs adam-web-ui/src/lib.rs
git commit -m "refactor(adam-web-ui): extract graph_drive_script from GraphView"
```

---

### Task 2: Preprocessor — fail the build when a `<graph>` has no paired inspector

**Files:**
- Modify: `adam-lang-book-preprocessor/src/main.rs` (extend `inject_graph_mount_points` + tests)

**Interfaces:**
- Consumes: nothing new. `inject_graph_mount_points(content, re, chapter_dir, examples_dir)` already receives the chapter content *after* `inject_mount_points` has run (see `run()` at `main.rs:219-221`), so `content` already contains the injected `<div class="adam-live" data-example="...">` inspector mounts.
- Produces: no new symbols; the same function gains a validation branch.

**Context for the implementer:** The book pairs each `{{#include examples/<chapter>/<name>.adm2}}` with an auto-inserted `<div class="adam-live" data-example="<chapter>/<name>">` (the inspector that owns the sheet). A `<graph sheet="name">` can only become dynamic if that inspector exists on the same page. The presence of that exact inspector div in the already-include-processed content is the precise signal.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang-book-preprocessor/src/main.rs`:

```rust
#[test]
fn inject_graph_mount_points_errors_when_no_paired_inspector_is_present() {
    let tmp = std::env::temp_dir().join(format!(
        "adam-lang-book-preprocessor-test-{}-e",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    write_example(&tmp, "tutorial", "first_sheet");

    let re = graph_tag_regex();
    // The example file exists on disk, but the chapter never includes it, so no inspector
    // mount will own its sheet — the graph could never become dynamic.
    let content = "prose\n\n<graph sheet=\"first_sheet\">\n\nmore prose";
    let result = inject_graph_mount_points(content, &re, "tutorial", &tmp);

    assert!(
        result.is_err(),
        "a <graph> with no paired inspector on the page must fail the build"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("first_sheet"),
        "error must name the unpaired graph: {err}"
    );
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn inject_graph_mount_points_accepts_a_graph_with_a_paired_inspector() {
    let tmp = std::env::temp_dir().join(format!(
        "adam-lang-book-preprocessor-test-{}-f",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    write_example(&tmp, "tutorial", "first_sheet");

    let re = graph_tag_regex();
    // Mirrors post-`inject_mount_points` content: the inspector div is already present.
    let content = "<div class=\"adam-live\" data-example=\"tutorial/first_sheet\"></div>\n\n<graph sheet=\"first_sheet\">";
    let result = inject_graph_mount_points(content, &re, "tutorial", &tmp).unwrap();

    assert!(result.contains(
        "<div class=\"adam-live-graph\" data-example=\"tutorial/first_sheet\"></div>"
    ));
    std::fs::remove_dir_all(&tmp).unwrap();
}
```

Also update the two existing tests that pass content with no inspector div — they must now include one so they still represent valid input:

In `inject_graph_mount_points_replaces_a_known_sheet_reference`, change the `content` line to:

```rust
        let content = "<div class=\"adam-live\" data-example=\"tutorial/first_sheet\"></div>\n\nprose\n\n<graph sheet=\"first_sheet\">\n\nmore prose";
```

In `inject_graph_mount_points_accepts_a_paired_closing_tag`, change the `content` line to:

```rust
        let content = "<div class=\"adam-live\" data-example=\"tutorial/first_sheet\"></div>\n<graph sheet=\"first_sheet\"></graph>";
```

(The two `_errors_when_the_example_does_not_exist` / `_reports_the_first_missing_reference` tests reference non-existent files, so they still fail on the file-existence check before the new pairing check — leave them unchanged.)

- [ ] **Step 2: Run the tests to verify the new/updated ones fail**

Run: `cargo test -p adam-lang-book-preprocessor inject_graph_mount_points`
Expected: `inject_graph_mount_points_errors_when_no_paired_inspector_is_present` FAILS (no error is returned yet); the two updated tests still pass (extra div is harmless until the check exists).

- [ ] **Step 3: Add the pairing validation**

In `inject_graph_mount_points` (`main.rs:175-200`), inside the `replace_all` closure, after the existing file-existence check and before returning the replacement `<div>`, add a paired-inspector check:

```rust
        let inspector_div = format!(
            "class=\"adam-live\" data-example=\"{chapter_dir}/{name}\""
        );
        if !content.contains(&inspector_div) {
            error.get_or_insert_with(|| {
                format!(
                    "<graph sheet=\"{name}\"> in chapter \"{chapter_dir}\" has no paired \
                     live example on the page: the chapter must {{{{#include}}}} \
                     examples/{chapter_dir}/{name}.adm2 so an inspector owns its sheet"
                )
            });
            return String::new();
        }
```

Note: `content` is already in scope (the function's `content: &str` parameter). Place this block after the `if !adm2_path.is_file() { ... }` block.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang-book-preprocessor`
Expected: PASS (all preprocessor tests, including the new pairing tests).

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-lang-book-preprocessor --all-targets -- -D warnings`
Expected: clean.

```bash
git add adam-lang-book-preprocessor/src/main.rs
git commit -m "feat(adam-lang-book-preprocessor): require a paired inspector for every <graph>"
```

---

### Task 3: `adam-lang-book-live` — own the sheet, drive the graph, drop `mount_graph`

**Files:**
- Modify: `adam-lang-book-live/src/lib.rs` (extend `mount`; add the graph-driving effect to `Root`; delete `GraphRoot`, `GraphRootProps`, and `mount_graph`)

**Interfaces:**
- Consumes: `adam_web_ui::graph_drive_script` (Task 1), `adam_web_ui::to_graph_data` (existing).
- Produces: `pub fn mount(element_id: &str, source: &str, name: &str, graph_ids: Vec<String>)` — the new `graph_ids` parameter lists the graph container ids this example's sheet drives. Task 4 (bootstrap) calls it.

**Context for the implementer:** `Root` (`lib.rs:42-82`) builds the sheet from source and renders `SheetInspector`. It uses a delicate conditional-hook pattern (calling `use_signal` inside a `match` arm) that is sound only because `build_sheet` is deterministic over unchanging props — read the doc comment at `lib.rs:29-41`. The new `use_memo`/`use_signal`/`use_effect` you add go in the **same** `Some((sheet, labels))` arm, following that same pattern; do not move them outside the match.

- [ ] **Step 1: Update `mount` and `RootProps`**

In `adam-lang-book-live/src/lib.rs`, change `RootProps` (`lib.rs:13-17`) to carry the graph ids:

```rust
#[derive(Clone, PartialEq, Props)]
struct RootProps {
    source: String,
    name: String,
    graph_ids: Vec<String>,
}
```

Change `mount` (`lib.rs:91-100`) to accept and forward `graph_ids`:

```rust
#[wasm_bindgen]
pub fn mount(element_id: &str, source: &str, name: &str, graph_ids: Vec<String>) {
    let props = RootProps {
        source: source.to_string(),
        name: format!("{name}.adm2"),
        graph_ids,
    };
    let vdom = VirtualDom::new_with_props(Root, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
```

Update `mount`'s doc comment to note that it also drives each id in `graph_ids` (the graph containers bound to this example) on every propagate.

- [ ] **Step 2: Add the graph-driving effect to `Root`**

In `Root` (`lib.rs:42-82`), update the imports at the top of the file:

```rust
use adam_web_ui::spectrum::SpTheme;
use adam_web_ui::{Renderer, SheetInspector, build_sheet, graph_drive_script, to_graph_data};
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;
```

Then, inside the `Some((sheet, labels))` arm of the `match outcome.sheet_labels` (currently `lib.rs:54-65`), add the graph memo, a first-drive flag, and the effect, so the arm reads:

```rust
        Some((sheet, labels)) => {
            let sheet = use_signal(|| sheet);
            let labels = use_signal(|| labels);
            let error = outcome.error.clone();

            let data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));
            let graph_ids = props.graph_ids.clone();
            let mut first_drive = use_signal(|| true);
            use_effect(move || {
                let is_first = *first_drive.peek();
                if is_first {
                    first_drive.set(false);
                }
                let call = if is_first { "init" } else { "update" };
                // Read `data` synchronously so this effect re-subscribes to it, then build every
                // script before the async block: the container ids are all driven with one shared
                // `init` on the first run (the graphs draw from initial state) and `update` after.
                let scripts: Vec<String> = graph_ids
                    .iter()
                    .map(|id| graph_drive_script(id, &data.read(), call))
                    .collect();
                spawn(async move {
                    for script in scripts {
                        let _ = document::eval(&script).await;
                    }
                });
            });

            rsx! {
                SheetInspector { sheet, labels, source_text, source_name }
                if let Some(err) = error {
                    pre { class: "adam-live-error", "{err}" }
                }
            }
        }
```

(The `None` arm — parse failure — is unchanged: it renders the diagnostic `<pre>` and drives no graph.)

- [ ] **Step 3: Delete the graph-only mount path**

Remove `GraphRootProps` (`lib.rs:102-107`), the `GraphRoot` component (`lib.rs:109-146`), and `mount_graph` (`lib.rs:148-170`) entirely — the inspector mount now owns and drives the graph, so a separate graph mount no longer exists.

- [ ] **Step 4: Build the wasm crate**

Run: `cargo build -p adam-lang-book-live`
Expected: builds with zero warnings. (Fix any unused-import warnings — e.g. if `GraphView` was imported only for the deleted `GraphRoot`, it must be dropped from the `use`; the Step 2 import line already excludes it.)

- [ ] **Step 5: Lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-lang-book-live --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add adam-lang-book-live/src/lib.rs
git commit -m "feat(adam-lang-book-live): drive graphs from the inspector's live sheet"
```

---

### Task 4: Bootstrap — wire graph container ids into the inspector mounts

**Files:**
- Modify: `adam-lang-book/book-src/theme/adam-live-bootstrap.js` (reorder mounts; drop the graph-mount pass)

**Interfaces:**
- Consumes: `mount(id, source, name, graphIds)` from Task 3 (the wasm export now takes a fourth argument, a JS array of strings for `Vec<String>`). `mount_graph` no longer exists.

- [ ] **Step 1: Assign graph container ids up front and build the example→ids map**

In `adam-live-bootstrap.js`, replace the destructuring of the wasm module (currently `line 71`) to drop `mount_graph`:

```javascript
  const [{ default: init, mount }, manifest] = await Promise.all(loaders);
```

Then replace the two `forEach` mount loops (`lines 74-96`) with an id-wiring pass followed by an inspector-mount pass:

```javascript
  // Assign each graph container its own id and index it by the example it shows, so each
  // inspector mount can be handed the ids of the graphs its sheet must drive. The graph divs
  // are now plain containers graph.js attaches to; the inspector mount owns the sheet.
  const graphIdsByExample = new Map();
  graphMounts.forEach((div, index) => {
    const name = div.dataset.example;
    const id = `adam-live-graph-${index}`;
    div.id = id;
    if (!graphIdsByExample.has(name)) graphIdsByExample.set(name, []);
    graphIdsByExample.get(name).push(id);
  });

  inspectorMounts.forEach((div, index) => {
    const name = div.dataset.example;
    const source = manifest[name];
    if (source === undefined) {
      console.error(`adam-live: no embedded source for "${name}"`);
      return;
    }
    const id = `adam-live-${index}`;
    div.id = id;
    const graphIds = graphIdsByExample.get(name) || [];
    mount(id, source, name, graphIds);
  });
```

- [ ] **Step 2: Update the module's header comment**

Adjust the top-of-file comment (`lines 1-2`) so it no longer says a `GraphView` is mounted into each `.adam-live-graph` div; instead, each `.adam-live-graph` div is a container that the inspector mount for the same `data-example` drives via `window.beginGraph`.

- [ ] **Step 3: Manual sanity check of the script**

There is no JS unit-test harness in this repo. Confirm by reading: `mount` is called with four arguments; `mountGraph` is no longer referenced anywhere; `graphIdsByExample` is keyed by `div.dataset.example` (the same `chapter/name` value both the inspector and graph divs carry); d3/graph.js are still loaded only when `graphMounts.length > 0` (that guard, `lines 60-70`, is unchanged).

- [ ] **Step 4: Commit**

```bash
git add adam-lang-book/book-src/theme/adam-live-bootstrap.js
git commit -m "feat(adam-lang-book): bind each live graph to its example's inspector mount"
```

---

### Task 5: End-to-end verification and full check suite

**Files:** none modified (verification only, plus a handoff doc).

- [ ] **Step 1: Run the full workspace build and test suite**

Run: `cargo build --workspace`
Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, zero compiler warnings (read the output; `-D warnings` clippy does not catch every plain-build warning).

- [ ] **Step 2: Run all three clippy invocations**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 3: Build the book and serve it**

Build the wasm bundle and copy assets exactly as the book build does (follow `xtask`'s `prepare-live-book-assets` and the `wasm-pack build` step used for the existing live examples — the same commands the CI docs workflow runs), then `mdbook build` the book from `adam-lang-book/`. Serve `book-dist/` locally (e.g. any static file server) and open the tutorial chapter.
Expected: `mdbook build` succeeds (it would fail if any `<graph>` lacked a paired include — that is Task 2's guard working).

- [ ] **Step 4: Confirm the graph updates live**

In the served tutorial page's §1.1, edit `first_sheet`'s inspector widgets (the `width`/`height` cells). Confirm the graph's cell values update and any derived relationship state (arrow direction, active branch, forced cells/relationships, the changed-cell pulse) updates to match — the same behavior `begin` shows. A successful `mdbook build` is NOT sufficient evidence per the repo's UI verification rule; you must see the graph move.

- [ ] **Step 5: Confirm two graphs on one page stay independent**

On a page rendering two live graphs (or by adding a second `<graph>` for another example temporarily), confirm editing one example's widgets updates only its own graph, and dragging a node in one graph never moves the other. Revert any temporary authoring change used for this check.

- [ ] **Step 6: Re-confirm `begin` is unaffected**

Use the `verifying-begin-ui` skill once more on `begin`: the graph renders, zoom/fit/show-inactive work, and switching examples resets the layout cleanly.

- [ ] **Step 7: Write the handoff doc**

Create `docs/superpowers/handoffs/2026-09-03-dynamic-live-adam-book-graphs-handoff.md` (or the repo's established handoff location under `docs/superpowers/`) summarizing what shipped (inspector-owned sheet drives graphs), what was deliberately left out (graph-node editing, structural edits, cross-page pairing — the spec's non-goals), and any follow-ups discovered during implementation. Follow the format of `docs/superpowers/2026-07-18-phase-3-handoff.md`.

- [ ] **Step 8: Commit the handoff**

```bash
git add docs/superpowers/handoffs/2026-09-03-dynamic-live-adam-book-graphs-handoff.md
git commit -m "docs(adam-lang-book): handoff for dynamic live book graphs"
```

---

## Self-Review Notes

- **Spec coverage:** Decision "inspector owns the sheet, drives the graph, graph's sheet eliminated" → Task 3. "Display-only graph" → enforced by not adding any write-back (all tasks). "Bound to same-page inspector by resolved example" → Task 4's `graphIdsByExample` keyed by `data-example`. "Preprocessor fails on unpaired graph" → Task 2. "graph-drive seam is one pure helper" → Task 1. "graph div is the container directly, no `-container` child" → Task 4 (the div's own id is used) + Task 3 (no `GraphView`/`GraphRoot` mounted there). "`GraphView`/`to_graph_data` otherwise unchanged" → Task 1 leaves `to_graph_data` untouched and only relocates `GraphView`'s eval string. Removed items (`mount_graph`, `GraphRoot`, graph-mount pass, inline `format!`) → Tasks 3 and 4. Testing section → Tasks 1, 2, and 5.
- **Type consistency:** `graph_drive_script(container_id: &str, data: &GraphData, call: &str) -> String` is defined in Task 1 and consumed with that exact signature in Tasks 1 (GraphView) and 3 (Root). `mount(element_id, source, name, graph_ids: Vec<String>)` defined in Task 3, called with a 4th array argument in Task 4. `data-example` is the shared `chapter/name` key across the inspector div (preprocessor `inject_mount_points`), the graph div (`inject_graph_mount_points`), and the bootstrap map.
- **Placeholder scan:** no TBD/TODO; every code step shows the actual code.
