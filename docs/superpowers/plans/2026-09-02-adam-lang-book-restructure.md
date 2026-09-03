# Adam Language Book Restructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure *The Adam Programming Language* book (`adam-lang-book/`) so the Tutorial
teaches concepts in the order source → filters → outputs → requirements → relationships →
relationships continued (destructuring/self-reference) → conditionals, reorder the deep-dive
chapters to match, use `source` cells wherever an example never derives the cell, remove every
runnable example whose whole point is a parse/resolve error, and split comments/doc-comments out
into a new "Lexical Conventions" chapter.

**Architecture:** This is a documentation-and-example restructuring, not a runtime change. No
crate other than `adam-lang-book` (prose, `.adm2` examples, and the tests that `include_str!`
them) is touched, except two one-line edits to `adam-lang-book-live-config` are explicitly ruled
out below (see Global Constraints). Every `.adm2` file lives under
`book-src/examples/<chapter-dir>/<name>.adm2` and is `{{#include}}`d into exactly the chapter
markdown file that owns it; a matching `#[test]` in `tests/<chapter>.rs` `include_str!`s the same
file and asserts on its behavior. Reordering a chapter in the book means reordering its entry in
`SUMMARY.md` — chapter *numbers* are prose (`# Chapter N: ...`, `[Chapter N](...)`, `N.M`
cross-refs), not derived from file order, so every renumbered chapter's own heading and every
other chapter's reference to it must be hand-edited.

**Tech Stack:** mdBook, Rust (`adam_lang`/`adam_rs`/`cel_parser`/`cel_std`), `.adm2` source files.

**Spec:** No separate spec document exists; this plan's Global Constraints section below is the
full spec, derived from the user's request and verified against `adam-lang`/`adam-rs` source and
tests during planning.

## Global Constraints

- **Final chapter order and numbers** (edit `SUMMARY.md` to this exact list; every chapter's own
  `# Chapter N: ...` heading and every cross-reference elsewhere must match):
  1. Tutorial — `tutorial.md`
  2. Sheets, Cells, and Types — `cells.md`
  3. Source Cells — `source.md`
  4. Expressions and Dependency Deduction — `expressions.md`
  5. Filters — Self-Correcting Cells — `filters.md`
  6. Outputs and Requirements — `outputs.md`
  7. Relationships and the Solver — `relationships.md`
  8. Relationships Continued: Destructuring and Self-Referencing Methods — `relationships-continued.md` (**new**)
  9. Conditionals — `conditionals.md`
  10. Lexical Conventions — `lexical-conventions.md` (**new**)
  11. Program Style — `style.md`
  - Appendix A: Reference Manual — `reference.md` (unnumbered in the TOC, as today)
- **`cell` → `source` conversion rule:** in every runnable example **except** those in
  `cells.md` and `style.md` (see next bullet), change `cell name...` to `source name...` for any
  cell that is **never** named as a `relationship` binding's output or a `conditional` branch's
  relationship output anywhere in that same `.adm2` file. A cell that *is* claimed as an output
  in even one binding/branch in the file must stay `cell` (a `source` cell can never be a
  binding's output — `Error::InvalidCellKind`). Do this per-file, not per-cell-name: the same
  logical cell (e.g. `width`) is `source` in one example and stays `cell` in another if that
  other example's relationships claim it.
- **Exception — `cells.md` and `style.md` keep plain `cell`:** `cells.md`'s own examples
  (`tuple_typed_cell.adm2`) demonstrate `cell`/type-grammar mechanics, not source/derived sheet
  behavior — leave them as `cell`. `style.md`'s `canonical_formatting.adm2` demonstrates the
  formatter, not sheet semantics — leave it as `cell`. Do not apply the source-conversion rule to
  either file's examples.
- **Deleted "bad" examples — the following 8 `.adm2` files and their `#[test]` functions are
  deleted outright** (not kept, not de-emphasized): `cells/no_forward_references.adm2`,
  `cells/type_mismatch_is_a_parse_error.adm2`, `expressions/initializer_sees_no_cells.adm2`,
  `filters/must_reference_underscore.adm2`, `filters/tuple_filter_not_supported.adm2`,
  `relationships/conflict_error.adm2`, `relationships/cycle_error.adm2`,
  `source/source_cannot_be_derived.adm2`. The rule each one illustrated is still described in
  prose, with the exact error message quoted inline (backtick span), but with **no** runnable
  `{{#include}}`d example demonstrating the failure.
- **One exception kept out of the visible book but not deleted:**
  `expressions/no_standard_library.adm2` and its test (`tests/expressions.rs::no_standard_library`)
  stay on disk unchanged — this example requires a parser built *without* `cel-std`, which is
  also why `adam-lang-book-live-config::NO_LIVE_MOUNT` already excludes it from live-mounting.
  Deleting it would require reworking that cross-crate exclusion list and its
  `adam-lang-book-preprocessor` test for no benefit. Simply remove its `{{#include}}` from
  `expressions.md` §4.2 and rewrite that section as prose-only (no runnable snippet). Do **not**
  touch `adam-lang-book-live-config/src/lib.rs` or `adam-lang-book-preprocessor/src/main.rs` in
  this plan.
- **Verified technical facts this plan's prose relies on** (already confirmed against
  `adam-lang`/`adam-rs` source and tests during planning — implementers should not need to
  re-derive these, but *should* re-verify any new example's actual output by running its test
  before finalizing prose numbers):
  - A **forced** cell is the output of a relationship with exactly one method: since there is no
    alternative, that cell is claimed every round regardless of strength. `Sheet::is_forced(id)`
    (`adam-rs/src/sheet.rs`) reports this; it is `false` for a cell whose relationship has 2+
    methods, even if strength currently picks the same direction every time.
  - `adam-web-ui`'s Inspector computes a cell's `disabled` display flag as
    `forced || (has_outputs && not relevant)` (`adam-web-ui/src/inspector.rs::cell_flags`) — a
    forced cell's widget is disabled in the shipped UI. This is a `begin`/`adam-web-ui`
    convention, not a language rule, and should be described as "a host UI commonly disables the
    widget for a forced cell" rather than as something `adam-lang`/`adam-rs` mandates.
  - A relationship's methods must all reference the same `inputs ∪ outputs` cell set
    (`Error::MismatchedMethodCells`, message: `"methods in a relationship must reference the
    same set of cells"`) and no two methods may share an identical `outputs` set
    (`Error::DuplicateMethodOutputs`, message: `"a method's outputs must be duplicate-free, and
    no two methods in a relationship may share an outputs set"`) — both enforced in
    `Sheet::add_relationship` (`adam-rs/src/sheet.rs`) and surfaced through `adam-lang`'s parser
    via `ParseError::new(e.to_string(), ...)` (`adam-lang/src/parser.rs:797-799`), so these exact
    strings are what a sheet author sees.
  - A cell that is both an input and an output of the *same* method (a **self-referencing**
    method) is explicitly allowed (`Sheet::add_relationship`'s own doc comment, `adam-rs/src/sheet.rs`).
    Each round, a self-referencing method reads its own cell's **source** value, never a
    previous round's derived value (`adam-rs/tests/integration.rs::self_ref_pressure_persists_without_rewriting_anchor`).
    Idempotence (`f(f(x)) == f(x)`) is **not** checked by the solver — it is a correctness
    obligation on the sheet author, not a static or runtime check. A non-idempotent
    self-referencing method (e.g. a literal swap) does not converge to a single well-defined
    corrected value.
  - `require` already applies to `cell`, `source`, and `out` identically; `filter` already
    applies to `cell`, `source`, and `out` identically. Both facts are already correctly stated
    in the current book and only need to be *cross-referenced* from the new Tutorial ordering,
    not re-verified.
- **Do not** invent new adam-lang/adam-rs behavior. Every new `.adm2` example's expected values
  in this plan were hand-computed from verified mechanics above; treat a test failure as a signal
  to re-derive the correct expected value (and fix the prose to match), not to change the
  runtime.
- **Blockquote (`> `) formatting convention:** throughout this plan, a block of `> `-prefixed
  lines under a "with this content:" instruction is this *plan document's* way of visually
  setting off prose to copy into the target chapter file — it is plain paragraph prose in that
  file, **not** a literal markdown blockquote. Copy the text with the leading `> ` stripped from
  each line. A fenced ```` ``` ```` code block, by contrast, is always literal (including the
  nested ```` ``` ```` fence inside a blockquote in Task 11 Step 6, which itself becomes a real
  fenced code block in the copied prose, containing the real `{{#include}}` directive).
- Run `cargo test -p adam-lang-book` after every task. Do not run the full `mdbook build` pipeline
  per-task (it requires the wasm/preprocessor staging steps in `adam-lang-book/README.md`); do run
  it once at the end (Task 15).
- `cargo fmt --all` has no effect on `.md`/`.adm2` files but must still be run before any commit
  that touches `tests/*.rs` (per repository CLAUDE.md).

---

### Task 1: `SUMMARY.md` — new table of contents

**Files:**
- Modify: `adam-lang-book/book-src/SUMMARY.md`

**Interfaces:**
- Produces: the chapter order and file list every later task's cross-references must match
  (see Global Constraints' numbered list).

- [ ] **Step 1: Rewrite `SUMMARY.md`**

```markdown
# Summary

[Introduction](intro.md)

- [A Tutorial Introduction](tutorial.md)
- [Sheets, Cells, and Types](cells.md)
- [Source Cells](source.md)
- [Expressions and Dependency Deduction](expressions.md)
- [Filters — Self-Correcting Cells](filters.md)
- [Outputs and Requirements](outputs.md)
- [Relationships and the Solver](relationships.md)
- [Relationships Continued: Destructuring and Self-Referencing Methods](relationships-continued.md)
- [Conditionals](conditionals.md)
- [Lexical Conventions](lexical-conventions.md)
- [Program Style](style.md)

---

- [Appendix A: Reference Manual](reference.md)
```

- [ ] **Step 2: Commit**

```bash
git add adam-lang-book/book-src/SUMMARY.md
git commit -m "docs(adam-lang-book): reorder table of contents for the new chapter sequence"
```

(`mdbook build` isn't run per-task per Global Constraints, so there's no build step here; later
tasks create the two new files this TOC references.)

---

### Task 2: Delete the 8 approved "bad" examples and their tests

Do this before rewriting the chapters that used them, so later tasks edit prose that no longer
references a soon-to-be-deleted file.

**Files:**
- Delete: `adam-lang-book/book-src/examples/cells/no_forward_references.adm2`
- Delete: `adam-lang-book/book-src/examples/cells/type_mismatch_is_a_parse_error.adm2`
- Delete: `adam-lang-book/book-src/examples/expressions/initializer_sees_no_cells.adm2`
- Delete: `adam-lang-book/book-src/examples/filters/must_reference_underscore.adm2`
- Delete: `adam-lang-book/book-src/examples/filters/tuple_filter_not_supported.adm2`
- Delete: `adam-lang-book/book-src/examples/relationships/conflict_error.adm2`
- Delete: `adam-lang-book/book-src/examples/relationships/cycle_error.adm2`
- Delete: `adam-lang-book/book-src/examples/source/source_cannot_be_derived.adm2`
- Modify: `adam-lang-book/tests/cells.rs` (remove `no_forward_references`,
  `type_mismatch_is_a_parse_error` tests)
- Modify: `adam-lang-book/tests/expressions.rs` (remove `initializer_sees_no_cells` test)
- Modify: `adam-lang-book/tests/filters.rs` (remove `must_reference_underscore`,
  `tuple_filter_not_supported` tests)
- Modify: `adam-lang-book/tests/relationships.rs` (remove `conflict_error`, `cycle_error` tests)
- Modify: `adam-lang-book/tests/source.rs` (remove `source_cannot_be_derived` test)

- [ ] **Step 1: Delete the 8 files**

```bash
rm adam-lang-book/book-src/examples/cells/no_forward_references.adm2
rm adam-lang-book/book-src/examples/cells/type_mismatch_is_a_parse_error.adm2
rm adam-lang-book/book-src/examples/expressions/initializer_sees_no_cells.adm2
rm adam-lang-book/book-src/examples/filters/must_reference_underscore.adm2
rm adam-lang-book/book-src/examples/filters/tuple_filter_not_supported.adm2
rm adam-lang-book/book-src/examples/relationships/conflict_error.adm2
rm adam-lang-book/book-src/examples/relationships/cycle_error.adm2
rm adam-lang-book/book-src/examples/source/source_cannot_be_derived.adm2
```

- [ ] **Step 2: Trim `tests/cells.rs` to only `tuple_typed_cell`**

```rust
//! Examples backing `book-src/cells.md` (Chapter 2). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn tuple_typed_cell() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/tuple_typed_cell.adm2"
        ))
        .unwrap();
    let point = parsed.cell_names["point"].0;
    let value = parsed
        .read::<cel_runtime::DynamicSequence>(point)
        .unwrap()
        .clone();
    assert_eq!(value.try_to_tuple::<(f64, f64)>().unwrap(), (0.0, 0.0));
}
```

- [ ] **Step 3: Trim `tests/expressions.rs` to only `no_standard_library`**

```rust
//! Examples backing `book-src/expressions.md` (Chapter 4). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book. `no_standard_library` is kept as a regression test
//! only — it needs a parser built without `cel-std`, so it is never `{{#include}}`d into the
//! chapter itself; see `NO_LIVE_MOUNT` in `adam-lang-book-live-config`.

#[test]
fn no_standard_library() {
    let mut parser =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new());
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/expressions/no_standard_library.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("min"));
}
```

- [ ] **Step 4: Trim `tests/filters.rs`** — remove the `must_reference_underscore` and
  `tuple_filter_not_supported` `#[test]` functions (the last two in the file); leave
  `write_never_filters`, `raw_value_never_lost`, `range_filter_kind`,
  `derived_cell_diagnosed_not_corrected`, `filter_on_an_out_cell` untouched for now (Task 6
  rewrites their bodies for the `source` conversion).

- [ ] **Step 5: Trim `tests/relationships.rs`** — remove the `conflict_error` and `cycle_error`
  `#[test]` functions; leave `shared_cell_example` untouched for now (Task 8 rewrites it) and
  remove `destructuring_binding` from this file entirely (Task 9 moves it to a new
  `tests/relationships_continued.rs`).

- [ ] **Step 6: Trim `tests/source.rs`** — remove the `source_cannot_be_derived` `#[test]`
  function; leave the other three untouched for now (Task 4 rewrites `basic_source`).

- [ ] **Step 7: Run the trimmed test suite**

```bash
cargo test -p adam-lang-book
```

Expected: compiles; the remaining tests pass (chapters still `{{#include}}` the now-deleted
files at this point, but `cargo test` doesn't render markdown, so that's fixed in later tasks,
not this one).

- [ ] **Step 8: Commit**

```bash
git add adam-lang-book/book-src/examples adam-lang-book/tests
git commit -m "test(adam-lang-book): delete the 8 error-demonstrating examples and their tests"
```

---

### Task 3: Tutorial chapter — full rewrite (`tutorial.md`)

**Files:**
- Modify: `adam-lang-book/book-src/tutorial.md`
- Create: `adam-lang-book/book-src/examples/tutorial/forced_and_self_ref_shadow.adm2`
- Modify: `adam-lang-book/book-src/examples/tutorial/first_sheet.adm2`
- Modify: `adam-lang-book/book-src/examples/tutorial/clamp_demo.adm2`
- Modify: `adam-lang-book/book-src/examples/tutorial/destructuring_demo.adm2`
- Modify: `adam-lang-book/book-src/examples/tutorial/area_with_requirement.adm2`
- Delete: `adam-lang-book/book-src/examples/tutorial/multiplication_triangle.adm2` → kept, see
  below (not deleted — reused, unchanged content, just resequenced in prose)
- Modify: `adam-lang-book/book-src/examples/tutorial/mode_demo.adm2`
- Modify: `adam-lang-book/tests/tutorial.rs`

**Interfaces:**
- Consumes: nothing outside this chapter.
- Produces: the Tutorial's own worked examples; no other chapter includes them.

- [ ] **Step 1: Convert `first_sheet.adm2` to `source`**

```text
sheet hello {
    source width: i32 = 1920;
    source height: i32 = 1080;
}
```

- [ ] **Step 2: Convert `clamp_demo.adm2` to `source`**

```text
sheet volume {
    source level: i32 = 50 filter clamp: 0..=100;
}
```

- [ ] **Step 3: Convert `destructuring_demo.adm2`'s inputs to `source`** (outputs stay `cell`:
  they're each claimed by the relationship's destructuring binding)

```text
sheet rect_demo {
    source width: f64 = 10.0;
    source height: f64 = 4.0;
    cell area: f64;
    cell perimeter: f64;

    relationship {
        (area, perimeter) := (width * height, 2.0 * (width + height));
    }
}
```

- [ ] **Step 4: Convert `area_with_requirement.adm2`'s inputs to `source`**

```text
sheet area_demo {
    source width: i32 = 10;
    source height: i32 = 20;

    out area: i32 := width * height require {
        not_too_big: area <= 300;
    };
}
```

- [ ] **Step 5: Convert `mode_demo.adm2`'s match cell to `source`** (`x`/`y` stay `cell`: both
  are claimed as a binding output in some branch)

```text
sheet mode_demo {
    source p: i32 = 0;
    cell x: f64 = 1.0;
    cell y: f64 = 2.0;

    conditional p {
        0i32 => {
            relationship {
                x := y;
            }
        }
        1i32 => {
            relationship {
                y := x;
            }
        }
        _ => {
            relationship {
                x := 0.0;
            }
        }
    }
}
```

- [ ] **Step 6: Create `forced_and_self_ref_shadow.adm2`** (new; walkthrough hand-computed in
  Global Constraints — re-verify by running its test in Step 9 below before finalizing prose
  numbers)

```text
sheet range_bounds {
    source mode: i32 = 0;
    cell low: i32 = 4;
    cell high: i32 = 9;

    conditional mode {
        0i32 => {
            relationship {
                low := min(low, high);
                high := max(low, high);
            }
        }
        1i32 => {
            relationship {
                low := high;
            }
        }
    }
}
```

- [ ] **Step 7: Leave `multiplication_triangle.adm2` unchanged** (`a`, `b`, `c` all get claimed
  as a binding output by some method in the relationship, so none can be `source`).

- [ ] **Step 8: Rewrite `tutorial.md`** to this exact structure (each `{{#include}}` path is
  fixed by the steps above; write the prose paragraphs — voice and rigor matching the existing
  chapter's style, e.g. "A **cell** is a named, typed storage location..." — do not paraphrase
  facts loosely):

  - **§1.1 A first sheet** — introduce `sheet`/`source` syntax via `first_sheet.adm2`. State the
    spreadsheet analogy explicitly: *a `source` cell is like a spreadsheet's value cell — a slot
    you type a number into directly, with nothing else in the sheet computing it for you.*
    Note `source` looks exactly like `cell` syntactically (type/initializer/semicolon) and that
    the distinction (always an input, never computed) matters once relationships exist —
    forward-reference §1.5. Cover semicolons/declaration-sequence-not-statement-sequence exactly
    as the current §1.1 does. Point to Appendix A.10 (was A.11) for the host embedding API,
    exactly as today.
  - **§1.2 Filters: self-correcting cells** — move up from the old §1.4. Use `clamp_demo.adm2`.
    State plainly this is an **inclusive** range filter (`0..=100` — both `0` and `100` are valid
    values) and that a host UI mounts this as a live, editable widget: invite the reader to try
    writing an out-of-range value in the live example on this page and watch it snap back into
    `[0, 100]` only once the sheet next resolves, never at the moment of the write. Keep the
    existing "writing never filters" / "raw value is never lost" one-paragraph summary (full
    treatment is Chapter 5); update the forward-reference from "Chapter 7" to "Chapter 5".
  - **§1.3 Outputs: read-only, computed cells** — new position (was §1.6, requirements split
    off). Introduce `out` via a **new**, requirements-free variant — reuse `basic_output.adm2`
    verbatim is not appropriate here since that file lives in `outputs.md`'s own examples
    directory; instead write the analogy in prose only, no new example needed yet, and forward
    to §1.4 for a worked one: *an `out` cell is like a spreadsheet's equation cell — you never
    type into it directly, and its value is always whatever its formula currently computes.*
    State the two hard rules plainly: nothing may ever write an `out` cell directly (not a host
    write, not a relationship, not another `out`), and it's recomputed exactly once every time
    the sheet resolves. Forward-reference Chapter 6.
  - **§1.4 Requirements** — use `area_with_requirement.adm2` (now `source`-converted). Keep the
    existing explanation (diagnostic, not a gate; failed requirement never blocks resolution).
    Add the two cross-cutting facts the user asked for explicitly: *`require` isn't limited to
    `out` — the same block can trail a `source` declaration too, see §2.4/Chapter 3;* and
    *`filter` isn't limited to plain cells either — the same clause can trail an `out`
    declaration, see Chapter 5 §5.6.* Forward-reference Chapter 6 §6.3 for the full rules.
  - **§1.5 Relationships: a cell that can be either a source or derived** — use
    `multiplication_triangle.adm2` (unchanged, still `cell` throughout). Reframe the existing
    prose ("Nothing here names *which* cell is the 'output'...") explicitly around the idea the
    user asked for: *a plain `cell` inside a `relationship` isn't fixed as a source or an output
    the way `source`/`out` are — which role it plays is decided fresh each time the sheet
    resolves, driven by strength (§1.5.1 below).* Keep the existing strength explanation
    (declaration order breaks ties before any write; a write promotes a cell to freshest).
    Add a new subsection stating the two structural rules on a relationship's methods, quoting
    the exact error text from Global Constraints: every method's `inputs ∪ outputs` must be the
    same set (`MismatchedMethodCells`, quote the message), and no two methods may share an
    identical output set (`DuplicateMethodOutputs`, quote the message) — verify against
    `multiplication_triangle.adm2`'s own three bindings that both rules hold (`c := a * b`,
    `a := c / b`, `b := c / a`: each method's `inputs ∪ outputs` is `{a, b, c}`; the three output
    sets `{c}`, `{a}`, `{b}` are pairwise distinct). Forward-reference Chapter 7 for strength's
    full treatment (shared cells across relationships, conflicts, cycles).
  - **§1.6 Relationships continued: destructuring and self-reference** — use
    `destructuring_demo.adm2` (now `source`-converted) for destructuring, keeping the existing
    explanation. Add a **new** paragraph introducing self-referencing methods: a binding may name
    the same cell on both sides of `:=`, e.g. (describe, don't need a new tutorial-level adm2 —
    forward-reference Chapter 8's own `self_referencing_method.adm2` for a full worked example,
    matching how §1.2/§1.4/etc. forward-reference deep chapters rather than duplicating every
    example) — state the idempotence obligation plainly: *the method's own job is to correct a
    value into whatever set the relationship enforces; if reapplying it to its own already-
    corrected output would change the value again, the "correction" was never well-defined in the
    first place. The solver never checks this — it's on the sheet author.* Forward-reference
    Chapter 8.
  - **§1.7 Conditionals** — use `mode_demo.adm2` (now `source`-converted) for the basic
    branch/default-branch syntax, keeping the existing explanation. Then use
    `forced_and_self_ref_shadow.adm2` for the new content the user asked for: define **forced**
    (a relationship with exactly one method has no alternative, so its output cell is claimed
    every round regardless of strength — unlike the freely-chosen `low`/`high` roles in §1.5's
    triangle); state the UI convention plainly: *a host UI commonly disables the editable widget
    for a forced cell, since writing it would have no lasting effect once the sheet re-resolves.*
    Walk through the two branches using the numbers computed in this task's Step 6/9: branch
    `mode == 1` forces `low` from `high`, shadowing `low`'s own untouched source; switching back
    to `mode == 0` recomputes both `low` and `high` fresh from each cell's own **source** (never
    from the stale forced value) via the self-referencing `min`/`max` pair. Point out this is the
    same "derived value never destroys the source" model §1.2's filter already showed. Remove
    the old §1.3's separate treatment (that content is now folded into this section) — do not
    leave a stray "§1.3 Conditionals" duplicate heading.
  - **Remove old §1.7 "Comments" entirely** — replace with a one-line forward reference: *Adam's
    comment and doc-comment syntax is covered in [Chapter 10](lexical-conventions.md), not here.*
  - **§1.8 Where to go next** — keep, updating "Chapter 2 onward" phrasing if needed (it's
    already chapter-number-agnostic prose, likely needs no change beyond re-reading it in
    context).

- [ ] **Step 9: Rewrite `tests/tutorial.rs`** to match the new/renamed examples. Full file:

```rust
//! Examples backing `book-src/tutorial.md` (Chapter 1). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn first_sheet() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/first_sheet.adm2"
        ))
        .unwrap();

    let width = parsed.cell_names["width"].0;
    assert_eq!(*parsed.read::<i32>(width).unwrap(), 1920);
}

#[test]
fn clamp_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/clamp_demo.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // still raw

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now clamped
}

#[test]
fn area_with_requirement() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/area_with_requirement.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let output = parsed.output_names["area"];
    assert!(parsed.cell_requirements_valid(output)); // 10 * 20 == 200 <= 300

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.cell_requirements_valid(output)); // 50 * 20 == 1000 > 300
}

#[test]
fn multiplication_triangle() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/multiplication_triangle.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
    );
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 6.0); // 2.0 * 3.0, derived

    parsed.write(b, 5.0).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 5.0); // just written
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 10.0); // 2.0 * 5.0, re-derived
}

#[test]
fn destructuring_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/destructuring_demo.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (area, perimeter) = (
        parsed.cell_names["area"].0,
        parsed.cell_names["perimeter"].0,
    );
    assert_eq!(*parsed.read::<f64>(area).unwrap(), 40.0); // 10.0 * 4.0
    assert_eq!(*parsed.read::<f64>(perimeter).unwrap(), 28.0); // 2.0 * (10.0 + 4.0)

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(area).unwrap(), 40.0);
    assert_eq!(*parsed.read::<f64>(perimeter).unwrap(), 28.0);
}

#[test]
fn mode_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!("../book-src/examples/tutorial/mode_demo.adm2"))
        .unwrap();

    let p = parsed.cell_names["p"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 2.0); // p == 0: x := y

    parsed.write(p, 2_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 0.0); // p matches no named branch: default
}

#[test]
fn forced_and_self_ref_shadow() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/forced_and_self_ref_shadow.adm2"
        ))
        .unwrap();
    let (mode, low, high) = (
        parsed.cell_names["mode"].0,
        parsed.cell_names["low"].0,
        parsed.cell_names["high"].0,
    );

    // mode == 0 (declared default): self-referencing branch. 4 <= 9 already: unchanged.
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 9);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.source::<i32>(high).unwrap(), 9);

    // mode == 1: `low` is forced from `high` (single-method relationship).
    parsed.write(high, 42_i32).unwrap();
    parsed.write(mode, 1_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(parsed.is_forced(low));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 42);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4); // low's own source, untouched

    // Back to mode == 0: both cells recomputed fresh from their own sources (4, 42),
    // not from the stale forced 42.
    parsed.write(mode, 0_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 42);
}
```

- [ ] **Step 10: Run the tutorial tests**

```bash
cargo test -p adam-lang-book --test tutorial
```

Expected: PASS. If `forced_and_self_ref_shadow`'s asserted numbers don't match actual output,
trust the actual output — fix the assertions and the corresponding walkthrough numbers in
`tutorial.md` §1.7 (Step 8) to match, per Global Constraints.

- [ ] **Step 11: Commit**

```bash
git add adam-lang-book/book-src/tutorial.md adam-lang-book/book-src/examples/tutorial adam-lang-book/tests/tutorial.rs
git commit -m "docs(adam-lang-book): rewrite Chapter 1 (Tutorial) in source/filters/outputs/relationships/conditionals order"
```

---

### Task 4: Sheets, Cells, and Types (`cells.md`) — drop bad examples

**Files:**
- Modify: `adam-lang-book/book-src/cells.md`

**Interfaces:**
- Consumes: nothing new.
- Produces: §2.x anchors other chapters link to (`#22-cell-declarations`,
  `#23-built-in-types-and-inference`, `#25-tuple-types`, `#26-names-and-declaration-order`) —
  keep these anchor-producing headings' wording identical so existing links elsewhere in the book
  keep resolving; only the two bad-example subsections lose their `{{#include}}`.

- [ ] **Step 1: In §2.3 ("Built-in types and inference")**, remove the
  `{{#include examples/cells/type_mismatch_is_a_parse_error.adm2}}` block and its lead-in
  sentence ("When both are present, they must agree exactly, or the sheet fails to parse:").
  Replace with: "When both are present, they must agree exactly, or the sheet fails to parse
  with a \`type mismatch: expected \`T\`, got \`U\`\` error (Appendix A.4)." — no code block.

- [ ] **Step 2: In §2.6 ("Names and declaration order")**, remove the
  `{{#include examples/cells/no_forward_references.adm2}}` block and its lead-in sentence.
  Replace the final paragraph with: "Referencing a name before its declaration — as a
  `relationship` binding's output, a dependency inside any expression, a `conditional`'s match
  subject, or a `filter`'s dependency — is an \`undeclared cell \`name\`\` error (Appendix A.3)."
  — no code block.

- [ ] **Step 3: Leave every other section of `cells.md` unchanged** — `tuple_typed_cell.adm2`
  stays `cell` (see Global Constraints exception); this chapter otherwise has no example needing
  the `source` conversion or renumbering (it's already Chapter 2 in both old and new order).

- [ ] **Step 4: Run the cells tests**

```bash
cargo test -p adam-lang-book --test cells
```

Expected: PASS (only `tuple_typed_cell` remains from Task 2, unaffected by this task's prose-only
edits).

- [ ] **Step 5: Commit**

```bash
git add adam-lang-book/book-src/cells.md
git commit -m "docs(adam-lang-book): drop runnable error examples from Chapter 2, describe rules in prose"
```

---

### Task 5: Source Cells (`source.md`) — drop bad example, convert `basic_source`

**Files:**
- Modify: `adam-lang-book/book-src/source.md`
- Modify: `adam-lang-book/book-src/examples/source/basic_source.adm2`
- Modify: `adam-lang-book/tests/source.rs`

- [ ] **Step 1: Convert `basic_source.adm2`'s `height` to `source`**

```text
sheet resize {
    source width: i32 = 1920;
    source height: i32 = 1080;

    out area := width * height;
}
```

- [ ] **Step 2: Rewrite §3.1's prose** — the current text ("`width` above is declared `source`,
  `height` a plain `cell`; both are ordinary inputs to the `out` that multiplies them.") no
  longer matches the file. Replace with: "Both `width` and `height` above are declared `source`:
  ordinary inputs to the `out` that multiplies them, with nothing in this sheet ever claiming
  either as a relationship or conditional output."

- [ ] **Step 3: In §3.2 ("Always a source, never derived")**, remove the
  `{{#include examples/source/source_cannot_be_derived.adm2}}` block and its lead-in sentence
  ("This is checked once, structurally, the moment the offending declaration is parsed —
  resolving the sheet is never reached:"). Replace with: "This is checked once, structurally, the
  moment the offending declaration is parsed — resolving the sheet is never reached; naming a
  `source` cell on a binding's left-hand side is rejected before the sheet can ever be resolved."
  — no code block. (Do not attempt to quote `Error::InvalidCellKind`'s exact text here — per
  `tests/source.rs`'s own existing comment, its Display text is deliberately kind-agnostic as of
  issue #166 and carries no case-specific detail worth quoting.)

- [ ] **Step 4: Leave §3.3 unchanged** except updating any "Chapter 7" cross-reference to
  filters to "Chapter 5" (filters moved).

- [ ] **Step 5: Update `tests/source.rs`'s `basic_source` test** for the new all-`source` file
  (behavior is unchanged — `height` was never a relationship output either way — so the test
  body itself needs no assertion changes, only re-confirm it still compiles/passes):

```bash
cargo test -p adam-lang-book --test source
```

Expected: PASS with no test-code changes needed beyond what Task 2 already trimmed.

- [ ] **Step 6: Commit**

```bash
git add adam-lang-book/book-src/source.md adam-lang-book/book-src/examples/source/basic_source.adm2
git commit -m "docs(adam-lang-book): convert basic_source's height to source, drop the bad example from Chapter 3"
```

---

### Task 6: Expressions (`expressions.md`) — drop bad examples, keep as Chapter 4

**Files:**
- Modify: `adam-lang-book/book-src/expressions.md`

- [ ] **Step 1: In §4.2 ("No standard library of its own")**, remove the
  `{{#include examples/expressions/no_standard_library.adm2}}` block and its lead-in sentence.
  Replace the paragraph with: "A parser built with a bare `OpLookup::new()` and no library
  installed can still parse and run every construct in this book except a function call — any
  attempt to call an undefined function fails to parse with an error naming the missing function.
  This book's own examples always install `cel-std` (see `support::parser` in
  `adam-lang-book`'s own source)." — no code block. (The backing test,
  `tests/expressions.rs::no_standard_library`, stays as a regression test per Global Constraints;
  do not delete it.)

- [ ] **Step 2: In §4.3 ("Cell initializers see no cells")**, remove the
  `{{#include examples/expressions/initializer_sees_no_cells.adm2}}` block and its lead-in
  sentence. Replace with: "Referencing any identifier that would otherwise name a cell is an
  \`undeclared cell \`name\`\` error, exactly as if the cell had never been declared at all — see
  Appendix A.3." — no code block.

- [ ] **Step 3: Leave §4.1, §4.4, §4.5 unchanged** other than renumbering any cross-references
  that point at chapters whose numbers changed (none do within this chapter's own body — verify
  by re-reading after edits).

- [ ] **Step 4: Run the expressions tests**

```bash
cargo test -p adam-lang-book --test expressions
```

Expected: PASS (only `no_standard_library` remains from Task 2).

- [ ] **Step 5: Commit**

```bash
git add adam-lang-book/book-src/expressions.md
git commit -m "docs(adam-lang-book): drop runnable error examples from Chapter 4"
```

---

### Task 7: Filters (`filters.md`) — move to Chapter 5, convert to `source`, drop bad examples

**Files:**
- Modify: `adam-lang-book/book-src/filters.md`
- Modify: `adam-lang-book/book-src/examples/filters/write_never_filters.adm2`
- Modify: `adam-lang-book/book-src/examples/filters/raw_value_never_lost.adm2`
- Modify: `adam-lang-book/book-src/examples/filters/range_filter_kind.adm2`
- Modify: `adam-lang-book/book-src/examples/filters/derived_cell_diagnosed_not_corrected.adm2`
- Modify: `adam-lang-book/book-src/examples/filters/filter_on_an_out_cell.adm2`
- Modify: `adam-lang-book/tests/filters.rs`

- [ ] **Step 1: Convert `write_never_filters.adm2`**

```text
sheet s {
    source level: i32 = 50 filter clamp: 0..=100;
}
```

- [ ] **Step 2: Convert `raw_value_never_lost.adm2`**

```text
sheet spring_back {
    source max: i32 = 100 filter clamp: 0..=200;
    source level: i32 = 50 filter clamp: 0..=max;
}
```

- [ ] **Step 3: Convert `range_filter_kind.adm2`**

```text
sheet s {
    source level: i32 = 50 filter clamp: 0..=100;
}
```

- [ ] **Step 4: Convert `derived_cell_diagnosed_not_corrected.adm2`'s `driver` only** (`bound` is
  claimed by the relationship, so it must stay `cell`)

```text
sheet diagnose_only {
    cell bound: i32 = 100 filter clamp: 0..=100;
    source driver: i32 = 500;

    relationship {
        bound := driver;
    }
}
```

- [ ] **Step 5: Convert `filter_on_an_out_cell.adm2`**

```text
sheet s {
    source width: i32 = 4;
    out area := width filter clamp: 0..=100;
}
```

- [ ] **Step 6: Renumber the chapter heading and every section** — `# Chapter 5: Filters —
  Self-Correcting Cells`; §5.1 Grammar (was 7.1), §5.2 Writing never filters (was 7.2), §5.3 The
  raw value is never lost (was 7.3), §5.4 Range filters (was 7.4), §5.5 Derived cells: diagnosed,
  never corrected (was 7.5), §5.6 A filter on an output cell (was 7.6). Update every internal
  `7.x`/`#7x-...` anchor reference to `5.x`/`#5x-...` accordingly.

- [ ] **Step 7: In §5.4 ("Range filters")**, add one sentence tying this to the live book: "This
  book's own live examples mount an editable widget bound to the filtered cell whose displayed
  `min`/`max` track the range's current live bounds — try the example in
  [§1.2](tutorial.md#12-filters-self-correcting-cells) of the Tutorial." Keep the rest of the
  section's technical content (structural recognition of `lo..=hi`, exemption from the
  must-reference-`_` rule) unchanged.

- [ ] **Step 8: Rewrite old §7.7 ("Errors") as §5.7**, dropping both `{{#include}}`s and the
  code blocks entirely. Replace with prose-only:

  > Two filter-declaration mistakes are caught while parsing the sheet, before it is ever
  > resolved: a non-range filter body that never references `_` fails with `` `filter must
  > reference `_`` ``, and a `filter` attached to a tuple-typed cell fails with `` `filter on a
  > tuple-typed cell is not yet supported` ``. At most one filter may be attached per cell.

- [ ] **Step 9: Update `tests/filters.rs`'s remaining 5 tests** — no assertion changes needed
  (converting `cell` to `source` doesn't change any of these examples' `read`/`write`/
  `propagate` behavior, since none of the converted cells were ever a relationship output), but
  re-run to confirm:

```bash
cargo test -p adam-lang-book --test filters
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add adam-lang-book/book-src/filters.md adam-lang-book/book-src/examples/filters
git commit -m "docs(adam-lang-book): move Filters to Chapter 5, convert examples to source, drop bad examples"
```

---

### Task 8: Outputs and Requirements (`outputs.md`) — move to Chapter 6, convert to `source`

**Files:**
- Modify: `adam-lang-book/book-src/outputs.md`
- Modify: `adam-lang-book/book-src/examples/outputs/basic_output.adm2`
- Modify: `adam-lang-book/book-src/examples/outputs/output_cell_can_be_referenced.adm2`
- Modify: `adam-lang-book/book-src/examples/outputs/requirement_diagnostic.adm2`
- Modify: `adam-lang-book/book-src/examples/outputs/multiple_requirements.adm2`

- [ ] **Step 1: Convert `basic_output.adm2`**

```text
sheet area_demo {
    source width: i32 = 10;
    source height: i32 = 20;

    out area := width * height;
}
```

- [ ] **Step 2: Convert `output_cell_can_be_referenced.adm2`**

```text
sheet s {
    source width: i32 = 10;
    out area := width * 2;
    out doubled_area := area * 2;
}
```

- [ ] **Step 3: Convert `requirement_diagnostic.adm2`**

```text
sheet area_demo {
    source width: i32 = 10;
    source height: i32 = 20;

    out area: i32 := width * height require {
        not_too_big: area <= 300;
    };
}
```

- [ ] **Step 4: Convert `multiple_requirements.adm2`**

```text
sheet bounds_demo {
    source x: i32 = 50;

    out clamped: i32 := x require {
        not_negative: clamped >= 0;
        not_too_big: clamped <= 100;
    };
}
```

- [ ] **Step 5: Renumber the chapter heading and sections** — `# Chapter 6: Outputs and
  Requirements`; §6.1 Grammar (was 8.1), §6.2 An output cell can be read anywhere, written
  nowhere (was 8.2), §6.3 Requirements: diagnostics, not gates (was 8.3), §6.4 Multiple
  requirements (was 8.4). Update every internal `8.x` anchor to `6.x`.

- [ ] **Step 6: In §6.3**, keep the existing cross-cutting paragraph about `require` applying to
  `cell`/`source` too, but update its chapter links: "[Chapter 2](cells.md#22-cell-declarations)"
  stays (Chapter 2 unchanged), "[Chapter 3](source.md)" stays (Chapter 3 unchanged). Add one new
  sentence per the user's explicit ask, cross-linking the other direction: "A `filter` clause is
  just as unrestricted — see [Chapter 5 §5.6](filters.md#56-a-filter-on-an-output-cell) for a
  filter attached to an `out` declaration."

- [ ] **Step 7: No test changes needed** — converting `cell` to `source` doesn't change any of
  these files' behavior (none of the converted cells were ever a relationship output).

```bash
cargo test -p adam-lang-book --test outputs
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add adam-lang-book/book-src/outputs.md adam-lang-book/book-src/examples/outputs
git commit -m "docs(adam-lang-book): move Outputs to Chapter 6, convert examples to source"
```

---

### Task 9: Relationships and the Solver (`relationships.md`) — move to Chapter 7, add rules, drop bad examples, split out destructuring

**Files:**
- Modify: `adam-lang-book/book-src/relationships.md`
- Modify: `adam-lang-book/tests/relationships.rs` (rename remaining test file's doc comment;
  destructuring test moves to Task 10's new file)

**Interfaces:**
- Consumes: exact error strings from Global Constraints (`MismatchedMethodCells`,
  `DuplicateMethodOutputs`).
- Produces: §7.x anchors Task 3 (`tutorial.md` §1.5) and Task 10 (`relationships-continued.md`)
  link to.

`shared_cell_example.adm2` needs no `cell`→`source` conversion: every one of `a`, `b`, `c`, `d`
is claimed as some method's output across the two relationships (verified in planning), so all
four correctly stay `cell`.

- [ ] **Step 1: Renumber the chapter heading and carried-over sections** — `# Chapter 7:
  Relationships and the Solver`; §7.1 Bindings are alternative methods (was 5.1), §7.2 Strength:
  who gets to stay a source (was 5.2). Update the §7.2 forward-reference from "Chapter 1's
  [§1.2]" to "Chapter 1's [§1.5](tutorial.md#15-relationships-a-cell-that-can-be-either-a-source-or-derived)".

- [ ] **Step 2: Insert a new §7.3 ("The rules a relationship's methods must satisfy")** between
  the renumbered §7.2 and the shared-cell example, with this content:

  > Every method in the same `relationship` must reference exactly the same set of cells — the
  > union of that method's own `inputs` and `outputs` — as every other method in that
  > relationship; violating this fails to parse with `` `methods in a relationship must reference
  > the same set of cells` ``. A relationship models one fixed group of related cells; its
  > methods differ only in which subset of that group they treat as the output, using an
  > "ignore an input" pattern — not in which cells they touch at all. [Chapter 1 §1.5](tutorial.md#15-relationships-a-cell-that-can-be-either-a-source-or-derived)'s
  > multiplication triangle satisfies this: all three of `c := a * b`, `a := c / b`, and
  > `b := c / a` reference the same `{a, b, c}`.
  >
  > A method's own `outputs` list must be duplicate-free, and no two methods in the same
  > relationship may claim an identical `outputs` set; violating either fails to parse with
  > `` `a method's outputs must be duplicate-free, and no two methods in a relationship may share
  > an outputs set` ``. The planner treats a method's output set as one indivisible claim, so two
  > methods claiming the same set would make that claim ambiguous. The triangle's three output
  > sets — `{c}`, `{a}`, `{b}` — are pairwise distinct, as required.
  >
  > A cell may appear in both a method's `inputs` and its own `outputs` — a **self-referencing**
  > method — which is explicitly allowed and has its own rules; see
  > [Chapter 8](relationships-continued.md).

- [ ] **Step 3: Renumber §5.3 → §7.4 ("A shared-cell example")** unchanged otherwise (the
  `diamond` sheet's own text and `.adm2` file need no edits — none of its cells qualify for the
  `source` conversion, per this task's header note).

- [ ] **Step 4: Rewrite §5.4 → §7.5 ("When no assignment exists")** dropping both `{{#include}}`s
  and their code blocks. Replace with prose-only:

  > Every relationship in a sheet must end up with exactly one selected binding once the sheet
  > resolves; if that's not possible, resolution fails instead of silently picking something
  > inconsistent. Two relationships that both, unconditionally, insist on writing the *same* cell
  > can never both be satisfied, and resolving fails with `` `no valid method assignment
  > (overconstrained)` `` (`Error::Conflict`).
  >
  > A subtler failure is a **cycle**: an assignment exists, but every valid choice of bindings
  > forms a closed loop with no cell left as a source anywhere in the loop, and resolving fails
  > with `` `selected methods form a cycle` `` (`Error::Cycle`). This happens when every
  > relationship in the loop has only one binding, leaving the solver no alternative to try;
  > giving even one relationship in the loop a second, cycle-breaking binding lets the solver
  > route around it instead.

- [ ] **Step 5: Remove old §5.5 ("Destructuring bindings") from this file entirely** — replace
  with a one-line forward reference at the end of the chapter: "Destructuring a binding's output
  across more than one cell, and a binding that references its own output cell, are covered next,
  in [Chapter 8](relationships-continued.md)." (Task 10 creates that chapter with the moved
  content.)

- [ ] **Step 6: Update `tests/relationships.rs`'s doc comment** to say "Chapter 7" instead of
  "Chapter 5", and remove the `destructuring_binding` test (moves to Task 10). The file should
  now contain only `shared_cell_example`.

- [ ] **Step 7: Run the relationships tests**

```bash
cargo test -p adam-lang-book --test relationships
```

Expected: PASS (only `shared_cell_example` remains).

- [ ] **Step 8: Commit**

```bash
git add adam-lang-book/book-src/relationships.md adam-lang-book/tests/relationships.rs
git commit -m "docs(adam-lang-book): move Relationships to Chapter 7, add method rules, drop bad examples"
```

---

### Task 10: New chapter — Relationships Continued (`relationships-continued.md`)

**Files:**
- Create: `adam-lang-book/book-src/relationships-continued.md`
- Move: `adam-lang-book/book-src/examples/relationships/destructuring_binding.adm2` →
  `adam-lang-book/book-src/examples/relationships-continued/destructuring_binding.adm2`
- Create: `adam-lang-book/book-src/examples/relationships-continued/self_referencing_method.adm2`
- Create: `adam-lang-book/tests/relationships_continued.rs`

**Interfaces:**
- Consumes: `source.rs`(is_source)/`Sheet::is_forced` naming from Global Constraints (used in
  prose, not this chapter's own tests).
- Produces: `relationships-continued.md#82-self-referencing-methods` and
  `#83-self-referencing-methods-must-be-idempotent`, linked from `tutorial.md` §1.6 and
  `conditionals.md`.

- [ ] **Step 1: Move `destructuring_binding.adm2`** (content unchanged — every cell in it,
  `width`/`height`/`area`/`perimeter`, keeps its existing kind: `width`/`height` were already
  plain `cell` in the original file and this chapter doesn't apply the source-conversion
  differently than `relationships.md` did — re-check: in the original file both are plain `cell`
  with no relationship claiming them, so per the global rule they convert to `source` too)

```bash
mkdir -p adam-lang-book/book-src/examples/relationships-continued
git mv adam-lang-book/book-src/examples/relationships/destructuring_binding.adm2 \
       adam-lang-book/book-src/examples/relationships-continued/destructuring_binding.adm2
```

Then edit the moved file to convert `width`/`height` to `source` (they're never a binding
output; `area`/`perimeter` stay `cell`, they're the destructured output):

```text
sheet rect_demo {
    source width: f64 = 10.0;
    source height: f64 = 4.0;
    cell area: f64;
    cell perimeter: f64;

    relationship {
        (area, perimeter) := (width * height, 2.0 * (width + height));
    }
}
```

- [ ] **Step 2: Create `self_referencing_method.adm2`** (behavior verified against
  `adam-rs/tests/integration.rs::self_ref_direct_clamp` during planning; `level` must stay `cell`
  — it's the relationship's own output):

```text
sheet clamp_via_relationship {
    cell level: i32 = 0;

    relationship {
        level := min(level, 0);
    }
}
```

- [ ] **Step 3: Write `relationships-continued.md`**:

```markdown
# Chapter 8: Relationships Continued: Destructuring and Self-Referencing Methods

## 8.1 Destructuring bindings

A binding's left-hand side can name more than one output cell by parenthesizing it, in which
case the right-hand side must be a tuple expression of matching arity, split element-wise:

```
{{#include examples/relationships-continued/destructuring_binding.adm2}}
```

`(a, b) := ...` and the one-element `(a,) := ...` (trailing comma mandatory, matching Rust's
own 1-tuple pattern) both destructure; a bare `a := ...` or the equivalent single parenthesized
`(a) := ...` (mere grouping, no comma) instead binds the right-hand side's *whole* result
(including a tuple-typed one) directly to the one named cell. Destructuring and direct-bind are
otherwise governed by the same type-matching rules as any other binding: each output's declared
type must structurally match what the expression actually produces, checked at parse time.

## 8.2 Self-referencing methods

A method's expression may reference the very cell it writes — a **self-referencing** method —
which [Chapter 7](relationships.md#73-the-rules-a-relationships-methods-must-satisfy) already
noted is explicitly allowed: a cell may appear in both a method's inputs and its own outputs.
Each time the sheet resolves, a self-referencing method reads its own cell's *source* value —
never a previous round's derived value — the same source/derived split
[Chapter 5](filters.md#53-the-raw-value-is-never-lost) already introduced for filters:

```
{{#include examples/relationships-continued/self_referencing_method.adm2}}
```

Writing `level` above never applies the clamp itself — exactly like a filter, the correction
happens live, the next time the sheet resolves, against `level`'s own raw value, and that raw
value survives underneath the clamp forever.

## 8.3 Self-referencing methods must be idempotent

A self-referencing method exists to correct its own cell into whatever set of values the
relationship enforces. That only makes sense if reapplying the method to its own already-corrected
output leaves it unchanged — `f(f(x)) == f(x)`. `min(level, 0)` above satisfies this: once
`level` is at most `0`, computing `min` of that value and `0` again produces the same value.

The solver never checks this — nothing about `add_relationship` inspects a method's function for
idempotence, and nothing about `propagate` would refuse to resolve a sheet whose self-referencing
method isn't idempotent. It is purely a correctness obligation on the sheet author. A
self-referencing binding built from a genuinely non-idempotent operation — a literal swap between
two cells' current values, for instance, rather than a one-sided correction like `min`/`max`/
`clamp` — has no single well-defined corrected value for the solver to settle on.
```

- [ ] **Step 4: Create `tests/relationships_continued.rs`**:

```rust
//! Examples backing `book-src/relationships-continued.md` (Chapter 8). See `src/lib.rs` for how
//! these `.adm2` files are wired into the book.

#[test]
fn destructuring_binding() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships-continued/destructuring_binding.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let (area, perimeter) = (
        parsed.cell_names["area"].0,
        parsed.cell_names["perimeter"].0,
    );
    assert_eq!(*parsed.read::<f64>(area).unwrap(), 40.0); // 10.0 * 4.0
    assert_eq!(*parsed.read::<f64>(perimeter).unwrap(), 28.0); // 2.0 * (10.0 + 4.0)
}

#[test]
fn self_referencing_method() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships-continued/self_referencing_method.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 5_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 0);

    parsed.write(level, 0_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 0); // already conformed: idempotent

    parsed.write(level, -3_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), -3); // already <= 0: unchanged
}
```

- [ ] **Step 5: Run the new test file**

```bash
cargo test -p adam-lang-book --test relationships_continued
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add adam-lang-book/book-src/relationships-continued.md \
        adam-lang-book/book-src/examples/relationships-continued \
        adam-lang-book/tests/relationships_continued.rs
git commit -m "docs(adam-lang-book): add Chapter 8, Relationships Continued (destructuring + self-reference)"
```

---

### Task 11: Conditionals (`conditionals.md`) — move to Chapter 9, add forced/shadow-state content

**Files:**
- Modify: `adam-lang-book/book-src/conditionals.md`
- Modify: `adam-lang-book/book-src/examples/conditionals/multi_cell_match_subject.adm2`
- Modify: `adam-lang-book/book-src/examples/conditionals/default_branch_and_spring_back.adm2`
- Create: `adam-lang-book/book-src/examples/conditionals/forced_and_self_ref_shadow.adm2`
- Modify: `adam-lang-book/tests/conditionals.rs`

- [ ] **Step 1: Convert `multi_cell_match_subject.adm2`'s `resample`/`constrain` to `source`**
  (`locked` stays `cell` — claimed as output in both branches)

```text
sheet resample_demo {
    source resample: bool = true;
    source constrain: bool = true;
    cell locked: bool = false;

    conditional resample && constrain {
        true => {
            relationship {
                locked := true;
            }
        }
        false => {
            relationship {
                locked := false;
            }
        }
    }
}
```

- [ ] **Step 2: Convert `default_branch_and_spring_back.adm2`'s `mode` to `source`** (`x` stays
  `cell` — claimed as output in the one named branch)

```text
sheet no_default {
    source mode: i32 = 0;
    cell x: f64 = 1.0;

    conditional mode {
        0i32 => {
            relationship {
                x := 100.0;
            }
        }
    }
}
```

- [ ] **Step 3: Create `forced_and_self_ref_shadow.adm2`** — identical content to Task 3's
  `tutorial/forced_and_self_ref_shadow.adm2` (same established pattern as
  `destructuring_binding.adm2`/`destructuring_demo.adm2`: the same sheet lives once per chapter
  that uses it, each with its own test):

```text
sheet range_bounds {
    source mode: i32 = 0;
    cell low: i32 = 4;
    cell high: i32 = 9;

    conditional mode {
        0i32 => {
            relationship {
                low := min(low, high);
                high := max(low, high);
            }
        }
        1i32 => {
            relationship {
                low := high;
            }
        }
    }
}
```

- [ ] **Step 4: Renumber the chapter heading and existing sections** — `# Chapter 9:
  Conditionals`; §9.1 Grammar (was 6.1), §9.2 The match subject (was 6.2, uses
  `multi_cell_match_subject.adm2`).

- [ ] **Step 5: Insert §9.3 ("Forced cells")** before the old default-branch section:

  > A relationship with exactly one method has no alternative binding to choose: its output cell
  > is claimed every time the sheet resolves, regardless of strength. Such a cell is **forced**
  > — `Sheet::is_forced` reports this, and it's `false` for a cell whose relationship has two or
  > more methods, even if strength happens to pick the same direction every round.
  >
  > A common convention in a host UI is to disable the editable widget for a forced cell:
  > writing it would have no lasting effect once the sheet next resolves, so there's nothing
  > useful for the user to type into. This is a UI convention, not a language rule — `adam-lang`
  > and `adam-rs` never disable anything themselves; a host is always free to accept the write
  > anyway (see §9.4 below for what happens if it does).

- [ ] **Step 6: Insert §9.4 ("Shadow state: forced and self-referencing cells")**, using
  `forced_and_self_ref_shadow.adm2` and the numbers verified in Task 3 Step 9/10 (reuse the exact
  same values — this is the same sheet):

  > [Chapter 5](filters.md#53-the-raw-value-is-never-lost)'s filters and
  > [Chapter 8](relationships-continued.md#82-self-referencing-methods)'s self-referencing
  > methods both keep a cell's own raw *source* value forever, underneath whatever *derived*
  > value a live correction currently computes. A forced cell works the same way: forcing it
  > shadows its own source, never overwrites it.
  >
  > ```
  > {{#include examples/conditionals/forced_and_self_ref_shadow.adm2}}
  > ```
  >
  > With `mode == 0` (the declared default), the self-referencing branch is active: `low` and
  > `high` start at `4` and `9`, already satisfying `low <= high`, so both are left unchanged.
  > Writing `high` to `42` and switching to `mode == 1` activates the single-method branch,
  > forcing `low` from `high`: `low` reads `42`, but its own *source* is still `4` — the write
  > that actually changed something (`high`) never touched `low`'s source at all. Switching back
  > to `mode == 0` recomputes both cells fresh from their own sources — `4` and `42` — via
  > `min`/`max`, not from the stale forced `42`: `low` reads back down to `4`, `high` stays at
  > `42`.

- [ ] **Step 7: Renumber old §6.3 ("The default branch") to §9.5, retitled "The default branch
  and reverting to source"** (heading becomes `## 9.5 The default branch and reverting to
  source`, producing the anchor `#95-the-default-branch-and-reverting-to-source` used elsewhere
  in this plan), keeping its content but
  reframing its closing paragraph explicitly as a case of the same shadow-state mechanism just
  introduced: replace "This is the same source/derived split [Chapter 7](filters.md) covers for
  filters..." with "This is the same shadow-state mechanism §9.4 just showed for a forced cell,
  triggered here by *no* branch matching at all rather than by switching branches: a relationship
  that stops being active can never have written a cell's source, so the cell has nothing to
  revert to except that untouched source."

- [ ] **Step 8: Renumber old §6.4 ("Nested and chained conditionals") to §9.6**, content
  unchanged.

- [ ] **Step 9: Rewrite `tests/conditionals.rs`** to add the new test and keep the two existing
  ones (bodies unchanged — the `source` conversions don't change behavior):

```rust
//! Examples backing `book-src/conditionals.md` (Chapter 9). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book.

#[test]
fn multi_cell_match_subject() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/multi_cell_match_subject.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let locked = parsed.cell_names["locked"].0;
    assert!(*parsed.read::<bool>(locked).unwrap());

    let resample = parsed.cell_names["resample"].0;
    parsed.write(resample, false).unwrap();
    parsed.propagate().unwrap();
    assert!(!*parsed.read::<bool>(locked).unwrap());
}

#[test]
fn default_branch_and_spring_back() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/default_branch_and_spring_back.adm2"
        ))
        .unwrap();
    let mode = parsed.cell_names["mode"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 100.0); // mode == 0: branch active
    assert!(!parsed.is_source(x));

    parsed.write(mode, 7_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 1.0); // no branch matches; x reverts to its
    // own declared default, not 100.0
    assert!(parsed.is_source(x));
}

#[test]
fn forced_and_self_ref_shadow() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/forced_and_self_ref_shadow.adm2"
        ))
        .unwrap();
    let (mode, low, high) = (
        parsed.cell_names["mode"].0,
        parsed.cell_names["low"].0,
        parsed.cell_names["high"].0,
    );

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 9);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.source::<i32>(high).unwrap(), 9);

    parsed.write(high, 42_i32).unwrap();
    parsed.write(mode, 1_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(parsed.is_forced(low));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 42);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4);

    parsed.write(mode, 0_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 42);
}
```

- [ ] **Step 10: Run the conditionals tests**

```bash
cargo test -p adam-lang-book --test conditionals
```

Expected: PASS. As in Task 3, if the new test's numbers don't match actual output, fix the
assertions and `conditionals.md` §9.4's walkthrough together to match reality.

- [ ] **Step 11: Commit**

```bash
git add adam-lang-book/book-src/conditionals.md adam-lang-book/book-src/examples/conditionals adam-lang-book/tests/conditionals.rs
git commit -m "docs(adam-lang-book): move Conditionals to Chapter 9, add forced-cell and shadow-state content"
```

---

### Task 12: New chapter — Lexical Conventions (`lexical-conventions.md`)

**Files:**
- Create: `adam-lang-book/book-src/lexical-conventions.md`
- Create: `adam-lang-book/book-src/examples/lexical-conventions/doc_comments.adm2`
- Create: `adam-lang-book/tests/lexical_conventions.rs`

This chapter absorbs: old `tutorial.md` §1.7 ("Comments", already removed by Task 3), old
`style.md` §9.1/§9.2 ("Comments"/"Doc comments", removed by Task 13), and old `reference.md`
§A.1 ("Lexical conventions", removed by Task 14) — combining a narrative treatment of comments
with the terser keyword/punctuation inventory that used to live only in the appendix.

- [ ] **Step 1: Create `doc_comments.adm2`** (new; demonstrates every comment form in one file,
  and that `format_sheet` preserves doc comments — mirrors `style.rs`'s existing
  `canonical_formatting` test pattern):

```text
//! A sheet describing a simple resize dialog.
sheet image_resize {
    /// The image's width in pixels, before any resampling.
    cell width_pixels: i32 = 1920; // a trailing comment
    /* a block comment,
       on several lines */
    cell height_pixels: i32 = 1080;
}
```

- [ ] **Step 2: Write `lexical-conventions.md`**:

```markdown
# Chapter 10: Lexical Conventions

An Adam source file is tokenized as Rust/CEL tokens (via `proc_macro2`): identifiers, integer
and float literals (with optional type suffixes), string literals, and punctuation, exactly as
[`cel-parser`'s own lexical grammar](../cel_parser/index.html) defines them. Adam adds exactly
one lexical extension on top of CEL's own conventions: doc comments.

## 10.1 Comments

`//` starts a line comment; `/* ... */` a block comment — the same two forms C, Rust, and CEL
all share:

```text
// a whole-line comment
cell width: i32 = 1920; // a trailing comment
/* a block comment, on one line or several */
```

## 10.2 Doc comments

`///` immediately before a `cell`, `source`, `relationship`, `conditional`, or `out`
declaration, and `//!` immediately before the `sheet` keyword itself, are Adam's own addition to
CEL's lexical grammar: doc comments, recovered by the language server and the formatter, and
otherwise inert — they carry no meaning when the sheet resolves:

```
{{#include examples/lexical-conventions/doc_comments.adm2}}
```

## 10.3 Keywords and reserved identifiers

**Keywords**: `sheet`, `cell`, `source`, `relationship`, `conditional`, `out`, `require`,
`filter`. None of these can be used as a cell or sheet name. `_` is not a keyword but is
reserved in two specific positions: a `conditional`'s default branch
(`_ => { ... }`, [Chapter 9 §9.5](conditionals.md#95-the-default-branch-and-reverting-to-source)),
and inside a `filter` expression (the candidate value,
[Chapter 5 §5.1](filters.md#51-grammar)); elsewhere it is an ordinary identifier.

## 10.4 Punctuation

`:` (type annotation), `=` (cell initializer), `:=` (binding/output body), `=>` (conditional
branch), `;` (declaration terminator), `,` (list separator), `{ }` (block delimiters), `( )`
(tuple/grouping delimiters).
```

- [ ] **Step 3: Create `tests/lexical_conventions.rs`**:

```rust
//! Examples backing `book-src/lexical-conventions.md` (Chapter 10). See `src/lib.rs` for how
//! these `.adm2` files are wired into the book.

#[test]
fn doc_comments_are_preserved_by_the_formatter() {
    let mut ast_parser = adam_lang::AdamAstParser::new();
    let sheet = ast_parser
        .parse_str(include_str!(
            "../book-src/examples/lexical-conventions/doc_comments.adm2"
        ))
        .unwrap();
    assert!(sheet.errors.is_empty());

    let formatted = adam_lang::format_sheet(&sheet);
    assert!(formatted.contains("//! A sheet describing a simple resize dialog."));
    assert!(formatted.contains("/// The image's width in pixels, before any resampling."));
}
```

- [ ] **Step 4: Run the new test file**

```bash
cargo test -p adam-lang-book --test lexical_conventions
```

Expected: PASS. If `format_sheet`'s exact output differs (e.g. comment placement/spacing), adjust
the two `assert!(formatted.contains(...))` substrings to what it actually preserves, rather than
asserting exact full-string equality — the point of this test is only that both doc comments
survive formatting, not the entire byte-for-byte layout `style.md` already covers.

- [ ] **Step 5: Commit**

```bash
git add adam-lang-book/book-src/lexical-conventions.md \
        adam-lang-book/book-src/examples/lexical-conventions \
        adam-lang-book/tests/lexical_conventions.rs
git commit -m "docs(adam-lang-book): add Chapter 10, Lexical Conventions"
```

---

### Task 13: Program Style (`style.md`) — move to Chapter 11, drop comments sections

**Files:**
- Modify: `adam-lang-book/book-src/style.md`

- [ ] **Step 1: Remove old §9.1 ("Comments") and §9.2 ("Doc comments") entirely** — both moved
  to Chapter 10 by Task 12.

- [ ] **Step 2: Renumber the chapter heading and remaining section** — `# Chapter 11: Program
  Style`; the surviving section (old §9.3, "Canonical formatting") becomes §11.1. Update its own
  internal cross-references if any (it currently has none pointing at other chapters' numbers).

- [ ] **Step 3: `canonical_formatting.adm2` needs no change** — per Global Constraints, `style.md`
  is exempt from the `source` conversion.

- [ ] **Step 4: Run the style tests**

```bash
cargo test -p adam-lang-book --test style
```

Expected: PASS, no test changes needed.

- [ ] **Step 5: Commit**

```bash
git add adam-lang-book/book-src/style.md
git commit -m "docs(adam-lang-book): move Program Style to Chapter 11, drop comments (moved to Chapter 10)"
```

---

### Task 14: Reference Manual (`reference.md`) — renumber, add new rules, drop stale examples

**Files:**
- Modify: `adam-lang-book/book-src/reference.md`

This is a mechanical renumbering plus targeted content additions/removals; no example files are
involved (the appendix has never `{{#include}}`d any `.adm2` file).

- [ ] **Step 1: Remove old §A.1 ("Lexical conventions") entirely** — its content now lives in
  Chapter 10; replace its old position with nothing (the appendix's first section becomes what
  was A.2).

- [ ] **Step 2: Renumber every remaining appendix section down by one**: A.2 Grammar → A.1, A.3
  Sheets and namespaces → A.2, A.4 Cells and source cells → A.3, A.5 The type registry → A.4, A.6
  Relationships and the solver → A.5, A.7 Conditionals → A.6, A.8 Filters → A.7, A.9 Outputs and
  requirements → A.8, A.10 Error messages → A.9, A.11 The host embedding API → A.10. Update every
  internal `#a{N}-...` anchor and every cross-chapter link elsewhere in the appendix that points
  at one of these by its old number.

- [ ] **Step 3: In renumbered §A.5 ("Relationships and the solver")**, add two new bullets (after
  the existing "Selection is driven by cell **strength**..." bullet) stating the two structural
  rules from Task 9's new §7.3, with the exact error strings:

  > - Every method in a relationship must reference the same `inputs ∪ outputs` cell set as
  >   every other method in that relationship, or resolution fails to parse with `` `methods in a
  >   relationship must reference the same set of cells` ``. See
  >   [Chapter 7 §7.3](relationships.md#73-the-rules-a-relationships-methods-must-satisfy).
  > - A method's own `outputs` must be duplicate-free, and no two methods in a relationship may
  >   share an identical `outputs` set, or resolution fails to parse with `` `a method's outputs
  >   must be duplicate-free, and no two methods in a relationship may share an outputs set` ``.
  >   See [Chapter 7 §7.3](relationships.md#73-the-rules-a-relationships-methods-must-satisfy).
  > - A cell may appear in both a method's inputs and its own outputs — a self-referencing method
  >   — which is explicitly allowed. See
  >   [Chapter 8](relationships-continued.md#82-self-referencing-methods).

  Update this section's existing chapter links (`[Chapter 5]` → `[Chapter 7]`) throughout its
  other bullets too.

- [ ] **Step 4: In renumbered §A.6 ("Conditionals")**, add one new bullet:

  > - A relationship with exactly one method is **forced**: its output cell is claimed every
  >   round, regardless of strength. `Sheet::is_forced` reports this. See
  >   [Chapter 9 §9.3](conditionals.md#93-forced-cells).

  Update this section's existing chapter link (`[Chapter 6]` → `[Chapter 9]`).

- [ ] **Step 5: In renumbered §A.7 ("Filters")**, update the chapter link (`[Chapter 7]` →
  `[Chapter 5]`) and its internal anchor references (`#74-range-filters` etc. → `#54-...` etc,
  matching Task 7's renumbering).

- [ ] **Step 6: In renumbered §A.8 ("Outputs and requirements")**, update chapter links
  (`[Chapter 8]` → `[Chapter 6]`, `[Chapter 2]` stays, `[Chapter 3]` stays) matching Task 8's
  renumbering.

- [ ] **Step 7: In renumbered §A.9 ("Error messages")**, add two rows to the table for the new
  relationship rules (keep every existing row — the deleted worked examples don't make the rules
  themselves stop existing):

  ```markdown
  | `methods in a relationship must reference the same set of cells` | a relationship's methods have different `inputs ∪ outputs` sets |
  | `a method's outputs must be duplicate-free, and no two methods in a relationship may share an outputs set` | two methods in one relationship claim the same `outputs` set, or one method repeats a cell in its own `outputs` |
  ```

- [ ] **Step 8: In renumbered §A.10 ("The host embedding API")**, add `is_forced` to the
  `Sheet` bullet's list of named operations: "... `read`, `write`, `propagate`, `is_source`,
  `is_forced`, `cell_kind`, `filter_*`, ...".

- [ ] **Step 9: Global pass** — re-read the entire file once more after all edits above and fix
  any remaining old-numbered chapter link (`relationships.md` was Chapter 5 → 7, `conditionals.md`
  was Chapter 6 → 9, `filters.md` was Chapter 7 → 5, `outputs.md` was Chapter 8 → 6, `style.md`
  was Chapter 9 → 11) or old appendix anchor (`#a{2..11}-...` → `#a{1..10}-...`) missed by the
  targeted steps above.

- [ ] **Step 10: No tests back this file** (the appendix has no runnable examples) — confirm the
  full suite still passes after the renumbering pass:

```bash
cargo test -p adam-lang-book
```

- [ ] **Step 11: Commit**

```bash
git add adam-lang-book/book-src/reference.md
git commit -m "docs(adam-lang-book): renumber Appendix A, document the relationship-method and forced-cell rules"
```

---

### Task 15: Final consistency pass and full build

**Files:**
- Modify (as needed): any file touched by Tasks 1–14 whose cross-reference this task's audit
  finds stale.

**Interfaces:**
- Consumes: every file this plan touched.
- Produces: a book that builds and a test suite that passes in full — the deliverable.

- [ ] **Step 1: Grep every chapter file for stale chapter-number references** — search
  `adam-lang-book/book-src/*.md` for the literal strings `Chapter 5`, `Chapter 6`, `Chapter 7`,
  `Chapter 8`, `Chapter 9` (both the bare word and inside `[Chapter N]` links) and manually verify
  each hit now names the *new* chapter at that number (per Global Constraints' numbered list),
  not a leftover reference to the old order. Fix any mismatch found.

- [ ] **Step 2: Grep for stale intra-file section anchors** — search for `#5`, `#6`, `#7`, `#8`,
  `#9` followed by a digit inside markdown link targets (e.g. `#52-`, `#83-`) across all chapter
  files and confirm each points at a section that actually exists with that exact heading slug
  after this plan's renumbering. Fix any mismatch found.

- [ ] **Step 3: Confirm no chapter file still `{{#include}}`s a path under a now-deleted or
  now-moved example directory** — specifically confirm no file references
  `examples/relationships/destructuring_binding.adm2` (moved to `relationships-continued/` in
  Task 10) or any of the 8 deleted files from Task 2.

- [ ] **Step 4: Run the full `adam-lang-book` test suite**

```bash
cargo test -p adam-lang-book
```

Expected: PASS, every test in every file listed in this plan.

- [ ] **Step 5: Run the full repository check suite** per repository `CLAUDE.md` (this PR touches
  `tests/*.rs` in `adam-lang-book`, so the workspace-wide gates apply):

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```

Expected: zero warnings from `build`/`test`, zero clippy findings.

- [ ] **Step 6: Build the book itself**, following `adam-lang-book/README.md`'s staging steps
  (from the repository root):

```bash
cargo install --path adam-lang-book-preprocessor --force
cd adam-lang-book-live && wasm-pack build --target web --release && cd ..
cargo run -p xtask -- prepare-live-book-assets
mdbook build adam-lang-book
```

Expected: `mdbook build` exits 0 with no broken-link/broken-anchor warnings (mdBook prints these
for any `[text](target)` it can't resolve — pay particular attention to the intra-file `#n.m-...`
anchors from Steps 1–2, since mdBook does not always fail the build on a bad anchor, only a bad
file target).

- [ ] **Step 7: Spot-check the rendered output** — open `adam-lang-book/book-dist/tutorial.html`,
  `relationships-continued.html`, `conditionals.html`, and `lexical-conventions.html` and confirm
  each new/moved chapter renders with its live-mount `<div>`s present (per `README.md`, "Confirmed
  working" pattern) for at least one example.

- [ ] **Step 8: Final commit** (only if Steps 1–3 found anything to fix; otherwise this task
  produces no diff beyond what Tasks 1–14 already committed):

```bash
git add -A adam-lang-book
git commit -m "docs(adam-lang-book): fix remaining stale cross-references found in final consistency pass"
```
