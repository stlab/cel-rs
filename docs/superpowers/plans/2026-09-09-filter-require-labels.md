# Filter and Requirement Label Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the mandatory name from `cell_filter` at every layer (grammar through the `adam_rs` public API), and make `requirement`'s name optional via a new `@identifier` leading marker, propagating both changes through every parser, printer, downstream crate, test, `.adm2` example, and doc page that encodes the old syntax.

**Architecture:** Two independent grammar/API changes land bottom-up: `adam_rs` (the runtime `Sheet` API) first, then `adam-lang` (both its CST parser `ast_parser.rs` and its compile-to-`Sheet` parser `parser.rs`, plus its formatter and typechecker), then downstream consumers (`ez-adam`, `adam-web-ui`, `begin`, `adam-lang-book`). Each task's own tests must pass before the next task begins, since later tasks call the signatures earlier tasks change.

**Tech Stack:** Rust, `cel-parser`/`proc_macro2` tokenization, `adam_rs::Sheet`, `adam-lang`'s two parsers (`ast_parser.rs`, `parser.rs`), `mdbook`.

**Spec:** [docs/superpowers/specs/2026-09-09-filter-require-labels-design.md](../specs/2026-09-09-filter-require-labels-design.md)

## Global Constraints

- New grammar: `cell_filter = "filter" expression.` and `requirement = [ "@" identifier ] expression ";".`
- `adam_rs::Sheet::add_filter` takes no name parameter at all; `Sheet::filter_name` is deleted.
- `adam_rs::Sheet::add_requirement`'s name parameter becomes `Option<&str>`; `RequirementData.name` becomes `Option<String>`; `Sheet::add_out`'s `requirements: Vec<(&str, Requirement)>` becomes `Vec<(Option<&str>, Requirement)>`. `Sheet::requirement_name`'s signature (`Option<&str>`) is unchanged.
- Every existing requirement label in a test or `.adm2` file is preserved as `@name`, not dropped — this is a syntax migration, not an editorial pass.
- `cargo fmt --all` before every commit (pre-commit hook enforces this). `cargo build --workspace` and `cargo test --workspace` must produce zero warnings.
- Where a task instructs a "compiler-driven sweep" (change a signature, then fix every resulting `cargo check` error), follow the stated rule exactly at every reported call site — do not guess or skip sites the compiler doesn't flag as an error, since an untyped literal like `"positive"` passed where `Option<&str>` is now expected is *always* a type error, never a silent miscompile.

---

## Task 1: `adam_rs` — remove the filter name entirely

**Files:**
- Modify: `adam-rs/src/filter.rs`
- Modify: `adam-rs/src/sheet.rs:601-638` (`add_filter`), `adam-rs/src/sheet.rs:640-645` (`filter_name`, deleted)
- Modify: `adam-rs/src/error.rs:102-108` (`InvalidFilter` doc)
- Test: `adam-rs/src/filter.rs` (inline `#[cfg(test)]` module), `adam-rs/src/sheet.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Produces: `pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error>` (was `add_filter(cell, name: impl Into<String>, filter: Filter)`). `Sheet::filter_name` no longer exists.

- [ ] **Step 1: Remove `FilterData::name` and every constructor's name initialization**

In `adam-rs/src/filter.rs`, remove the `name` field from `FilterData`:

```rust
/// Internal storage for a single filter.
pub(crate) struct FilterData {
    /// The `TypeId` of the value this filter operates on, validated against its cell's
    /// registered type by `add_filter`.
    pub(crate) value_type: TypeId,
    /// Dynamic argument cells, resolved via `effective()` wherever the filter runs.
    pub(crate) args: Vec<CellId>,
    pub(crate) arg_types: Vec<TypeId>,
    pub(crate) function: FilterFn,
    /// What shape of validation/derivation this filter performs, beyond `function` — see
    /// [`FilterKind`]. Purely informational; never consulted by `write`/`propagate`/`add_filter`.
    #[allow(dead_code)]
    pub(crate) kind: FilterKind,
}
```

Remove the `name: String::new(),` line from both `Filter::new` (around line 78) and `Filter::range` (around line 188) — each becomes:

```rust
        debug_assert_eq!(args.len(), arg_types.len());
        Filter(FilterData {
            value_type,
            args,
            arg_types,
            function: Box::new(f),
            kind: FilterKind::Opaque,
        })
```

and

```rust
        debug_assert_eq!(args.len(), arg_types.len());
        Filter(FilterData {
            value_type,
            args,
            arg_types,
            function: Box::new(clamp),
            kind: FilterKind::Range {
                bounds: Box::new(bounds),
            },
        })
```

- [ ] **Step 2: Update `Sheet::add_filter` and delete `Sheet::filter_name`**

In `adam-rs/src/sheet.rs`, replace `add_filter`'s signature and body (lines 601-638):

```rust
    /// Attaches `filter` to `cell`.
    ///
    /// Never evaluates `filter`'s function — attaching a filter is not a fresh
    /// external input, so it never changes `cell`'s current effective value. The next
    /// full [`Sheet::propagate`] call conforms `cell` via the planner's
    /// `PlanStep::FilterReclamp` step; until then, `read()` reflects whatever `cell`
    /// held before this call.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
    ///   live cell in this sheet.
    /// - `Error::InvalidFilter` — `cell` already has a filter, `filter`'s own value
    ///   type does not match `cell`'s registered type, or `filter`'s argument list
    ///   names `cell` itself.
    /// - `Error::TypeMismatch` — an argument cell's registered type does not match the
    ///   type `filter` declared for it.
    ///
    /// - Complexity: O(a) where a is the number of `filter`'s argument cells.
    pub fn add_filter(&mut self, cell: CellId, mut filter: Filter) -> Result<(), Error> {
        let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
        if self.cells[cell].filter.is_some() {
            return Err(Error::InvalidFilter);
        }
        if filter.0.value_type != cell_type {
            return Err(Error::InvalidFilter);
        }
        if filter.0.args.contains(&cell) {
            return Err(Error::InvalidFilter);
        }
        for (&arg_id, &declared) in filter.0.args.iter().zip(filter.0.arg_types.iter()) {
            let arg_cell = self.cells.get(arg_id).ok_or(Error::InvalidId)?;
            if arg_cell.type_id != declared {
                return Err(Error::TypeMismatch {
                    expected: arg_cell.type_id,
                    found: declared,
                    location: None,
                });
            }
        }

        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }
        self.cells[cell].filter = Some(filter.0);
        Ok(())
    }
```

Delete the `filter_name` method (lines 640-645) entirely:

```rust
    /// Returns the name of `id`'s filter, if it has one.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, or has no filter.
    pub fn filter_name(&self, id: CellId) -> Option<&str> {
        self.cells.get(id)?.filter.as_ref().map(|f| f.name.as_str())
    }
```

- [ ] **Step 3: Update `Error::InvalidFilter`'s doc comment**

In `adam-rs/src/error.rs`, lines 102-108:

```rust
    /// An `add_filter` call is structurally invalid: the cell already has a filter,
    /// the filter's own value type does not match the cell's registered type, or the
    /// filter's own argument list names `cell` itself. (An unknown cell or an
    /// argument-cell type mismatch use the shared `InvalidId`/`TypeMismatch` variants
    /// instead — `add_filter` has no cell-kind restriction, so it never returns
    /// `InvalidCellKind`.)
    InvalidFilter,
```

- [ ] **Step 4: Delete the three tests whose entire premise is the removed name**

In `adam-rs/src/sheet.rs`'s test module, delete these three tests outright (their premise — a filter has a name — no longer exists):

- `add_filter_stores_and_reports_its_name` (around line 2992)
- `filter_name_returns_none_for_an_unfiltered_cell` (around line 3006)
- `add_filter_returns_invalid_filter_for_an_empty_name` (around line 3013)

- [ ] **Step 5: Compiler-driven sweep of every remaining `add_filter` call site in `adam-rs`**

Run:

```bash
cargo check -p adam-rs --all-targets
```

For every "this function takes 2 arguments but 3 were supplied" error, delete the middle `"..."` string-literal argument at the reported call site — e.g. `sheet.add_filter(a, "test_filter", Filter::from_fn_0(...))` becomes `sheet.add_filter(a, Filter::from_fn_0(...))`. This affects (non-exhaustively — let the compiler's own file:line output be authoritative) roughly 40 call sites across `adam-rs/src/sheet.rs`'s own test module, `adam-rs/src/planner.rs`, and `adam-rs/src/planner/digraph.rs`.

Repeat `cargo check -p adam-rs --all-targets` until it passes with zero errors.

- [ ] **Step 6: Run the full `adam-rs` test suite**

Run: `cargo test -p adam-rs`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/filter.rs adam-rs/src/sheet.rs adam-rs/src/error.rs adam-rs/src/planner.rs adam-rs/src/planner/digraph.rs
git commit -m "$(cat <<'EOF'
refactor(adam-rs): remove the name parameter from Sheet::add_filter

A cell can have at most one filter, so a per-filter name never
disambiguated anything the cell's own identity didn't already give a
caller — confirmed by call-graph analysis showing zero production
callers of Sheet::filter_name. Deletes FilterData::name,
Sheet::add_filter's name parameter, and Sheet::filter_name outright.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `adam_rs` — make the requirement name optional

**Files:**
- Modify: `adam-rs/src/requirement.rs:94-99` (`RequirementData`)
- Modify: `adam-rs/src/sheet.rs:475-534` (`add_requirement`), `:557-580` (`add_out`), `:750-755` (`requirement_name`)
- Modify: `adam-rs/src/error.rs:110-113` (`InvalidRequirement` doc)
- Test: `adam-rs/src/sheet.rs` (inline test module), `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: nothing from Task 1 (independent surface).
- Produces: `pub fn add_requirement(&mut self, cell: CellId, name: Option<&str>, requirement: Requirement) -> Result<RequirementId, Error>` (was `name: impl Into<String>`). `pub fn add_out(&mut self, writer: Method, requirements: Vec<(Option<&str>, Requirement)>) -> Result<CellId, Error>` (was `Vec<(&str, Requirement)>`). `Sheet::requirement_name`'s signature is unchanged (`Option<&str>`).

- [ ] **Step 1: Make `RequirementData.name` optional**

In `adam-rs/src/requirement.rs`, line 95:

```rust
/// Internal storage for a single requirement.
pub(crate) struct RequirementData {
    pub(crate) name: Option<String>,
    pub(crate) cell: CellId,
    pub(crate) inputs: Vec<CellId>,
    pub(crate) function: RequirementFn,
}
```

- [ ] **Step 2: Update `Sheet::add_requirement`**

In `adam-rs/src/sheet.rs`, replace lines 475-534:

```rust
    /// Attaches `requirement` to `cell`, labeled `name` if given.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `requirement`'s input cells, is not a
    ///   live cell in this sheet.
    /// - `Error::TypeMismatch` — an input's declared type does not match its cell's
    ///   registered type.
    /// - `Error::InvalidRequirement` — `name` is `Some` and `cell` already has a
    ///   requirement with that same name, or (`Cell`/`Source` kind only) evaluating
    ///   `requirement` against the referenced cells' current effective values
    ///   returns `Ok(false)`.
    /// - `Error::MethodFailed` — (`Cell`/`Source` kind only) evaluating `requirement`
    ///   against current values returns `Err`.
    ///
    /// - Complexity: O(k) where k is `requirement`'s input count.
    pub fn add_requirement(
        &mut self,
        cell: CellId,
        name: Option<&str>,
        requirement: Requirement,
    ) -> Result<RequirementId, Error> {
        let cell_data = self.cells.get(cell).ok_or(Error::InvalidId)?;
        if let Some(name) = name
            && cell_data
                .requirements
                .iter()
                .any(|&rid| self.requirements[rid].name.as_deref() == Some(name))
        {
            return Err(Error::InvalidRequirement);
        }
        if requirement.inputs.len() != requirement.input_types.len() {
            return Err(Error::InvalidRequirement);
        }
        for (&input_id, &declared) in requirement
            .inputs
            .iter()
            .zip(requirement.input_types.iter())
        {
            let input_cell = self.cells.get(input_id).ok_or(Error::InvalidId)?;
            if input_cell.type_id != declared {
                return Err(Error::TypeMismatch {
                    expected: input_cell.type_id,
                    found: declared,
                    location: None,
                });
            }
        }

        if self.cells[cell].kind != CellKind::Out {
            let inputs: Vec<&dyn Any> = requirement
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (requirement.function)(&inputs).map_err(|error| Error::MethodFailed {
                error,
                location: None,
            })?;
            if !holds {
                return Err(Error::InvalidRequirement);
            }
        }

        let rid = self.requirements.insert(RequirementData {
            name: name.map(str::to_string),
            cell,
            inputs: requirement.inputs,
            function: requirement.function,
        });
        self.cells[cell].requirements.push(rid);
        Ok(rid)
    }
```

- [ ] **Step 3: Update `Sheet::add_out`'s `requirements` parameter type**

In `adam-rs/src/sheet.rs`, line 560, change the signature:

```rust
    pub fn add_out(
        &mut self,
        writer: Method,
        requirements: Vec<(Option<&str>, Requirement)>,
    ) -> Result<CellId, Error> {
```

(The body's `for (name, requirement) in requirements { self.add_requirement(out_cell, name, requirement)?; }` loop at lines 575-577 needs no change — `name` is already `Option<&str>`, matching the new `add_requirement` signature.)

- [ ] **Step 4: Update `Sheet::requirement_name`'s body**

In `adam-rs/src/sheet.rs`, lines 753-755:

```rust
    pub fn requirement_name(&self, id: RequirementId) -> Option<&str> {
        self.requirements.get(id)?.name.as_deref()
    }
```

- [ ] **Step 5: Update `Error::InvalidRequirement`'s doc comment**

In `adam-rs/src/error.rs`, lines 110-113:

```rust
    /// An `add_requirement` call is structurally invalid: `name` is `Some` and `cell`
    /// already has a same-named requirement, or (on a `Cell`/`Source` kind cell)
    /// evaluating the requirement against current values returns `Ok(false)`.
    InvalidRequirement,
```

- [ ] **Step 6: Delete the one test whose entire premise is the removed empty-name error, and update the duplicate-name test**

In `adam-rs/src/sheet.rs`'s test module, delete `add_requirement_returns_invalid_requirement_for_empty_name` (around line 3681) outright.

Update `add_requirement_returns_invalid_requirement_for_duplicate_name_on_same_cell` (around line 3689) to pass `Some(...)`:

```rust
    #[test]
    fn add_requirement_returns_invalid_requirement_for_duplicate_name_on_same_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_requirement(
                a,
                Some("positive"),
                Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
            )
            .unwrap();
        let result = sheet.add_requirement(
            a,
            Some("positive"),
            Requirement::from_fn_1(a, |x: &i32| Ok(*x < 100)),
        );
        assert!(matches!(result, Err(Error::InvalidRequirement)));
    }
```

Add one new test confirming two unlabeled requirements on the same cell do *not* collide (the new behavior the duplicate-name check must now support):

```rust
    #[test]
    fn add_requirement_allows_two_unnamed_requirements_on_the_same_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_requirement(a, None, Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)))
            .unwrap();
        let result = sheet.add_requirement(a, None, Requirement::from_fn_1(a, |x: &i32| Ok(*x < 100)));
        assert!(result.is_ok());
    }
```

- [ ] **Step 7: Compiler-driven sweep of every remaining `add_requirement`/`add_out` call site in `adam-rs`**

Run:

```bash
cargo check -p adam-rs --all-targets
```

For every "mismatched types: expected `Option<&str>`, found `&str`" (or `Vec<(&str, Requirement)>` vs `Vec<(Option<&str>, Requirement)>`) error, apply this rule at the reported call site:

- A string-literal name argument, e.g. `sheet.add_requirement(a, "positive", req)` → wrap it: `sheet.add_requirement(a, Some("positive"), req)`.
- A `Vec::<(&str, Requirement)>::new()` (the empty-requirements case for `add_out`) → `Vec::<(Option<&str>, Requirement)>::new()`.
- A `vec![("name", req), ...]` literal passed to `add_out` → `vec![(Some("name"), req), ...]`.

This affects `adam-rs/src/sheet.rs`'s own test module (e.g. `cell_has_the_requirement_it_was_given`, `add_out_returns_the_cell_id_directly`, and every other `add_out`/`add_requirement` test) and `adam-rs/tests/integration.rs`. In `adam-rs/tests/integration.rs` specifically:

- `add_out_returns_invalid_requirement_for_duplicate_requirement_names` (line 1453): both `"check"` literals become `Some("check")`.
- `add_out_returns_invalid_requirement_for_empty_requirement_name` (line 1474): this test's entire premise (an empty-string name is an error) no longer exists — **delete it outright**, matching Task 1/Step 4's treatment of the filter equivalent.
- Every other `add_out`/`add_requirement` call site (e.g. `add_out_succeeds_with_one_requirement`, `cell_requirements_returns_requirement_ids_in_declaration_order`, `requirement_cell_and_inputs_return_correct_values`, `violated_requirements_returns_only_the_failing_subset_of_multiple_requirements`) follows the same mechanical wrap-in-`Some` rule.
- `requirement_name_cell_inputs_return_none_for_invalid_id` (line 1631) and `violated_requirements_lists_the_failing_requirement` (line 1665) need **no change** — they only ever *read* `requirement_name`'s return value (`Option<&str>`, unchanged), never construct a name argument.

Repeat `cargo check -p adam-rs --all-targets` until it passes with zero errors.

- [ ] **Step 8: Run the full `adam-rs` test suite**

Run: `cargo test -p adam-rs`
Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add adam-rs/src/requirement.rs adam-rs/src/sheet.rs adam-rs/src/error.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
refactor(adam-rs): make Sheet::add_requirement's name optional

Applications do consume a requirement's name for reporting (e.g.
adam-web-ui's compute_output_status joins violated requirement names
for display), so unlike a filter's name this one stays — but requiring
every requirement to be named, uniqueness-checked, non-empty is
unnecessary friction when an application never surfaces it.
RequirementData.name and Sheet::add_requirement/add_out's name
parameters become Option<String>/Option<&str>; Sheet::requirement_name
now also returns None for a live-but-unlabeled requirement, which its
one production consumer already treats correctly via filter_map.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `adam-lang` — AST field changes and top-level grammar doc

**Files:**
- Modify: `adam-lang/src/ast.rs:259-271` (`CellFilter`), `:386-407` (`RequirementDecl`), `:751-778` (test)
- Modify: `adam-lang/src/lib.rs:13`, `:25` (grammar doc)

**Interfaces:**
- Consumes: nothing (pure data-shape change; the parsers that populate these fields are Tasks 4/5).
- Produces: `CellFilter { body, span }` (no `name`/`name_span`). `RequirementDecl { name: Option<String>, name_span: Option<ExprSpan>, body, leading_comment, blank_line_before, span }`.

- [ ] **Step 1: Update `CellFilter`**

In `adam-lang/src/ast.rs`, lines 259-271:

```rust
/// `cell_filter = "filter" expression.`
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's body expression. `_` inside it denotes the candidate value being conformed;
    /// every other identifier that names an already-declared cell is a deduced dependency.
    pub body: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}
```

- [ ] **Step 2: Update `RequirementDecl`**

In `adam-lang/src/ast.rs`, lines 386-407:

```rust
/// `requirement = [ "@" identifier ] expression ";".`
///
/// `name`, when present, is a plain string label passed to `adam_rs::Sheet::add_requirement`,
/// not a cell reference — it may coincide with a cell name declared elsewhere in the sheet but
/// doesn't have to.
#[derive(Debug, Clone)]
pub struct RequirementDecl {
    /// The requirement's declared label, if the `@identifier` marker was present.
    pub name: Option<String>,
    /// The `@identifier` marker's span, if present.
    pub name_span: Option<ExprSpan>,
    /// The parsed requirement body expression; must type-check as `bool`.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// Whether a blank line preceded this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `[ "@" identifier ] expr;` declaration.
    pub span: ExprSpan,
}
```

- [ ] **Step 3: Fix the one struct-literal test in `ast.rs`**

In `adam-lang/src/ast.rs`, update `cell_decl_filter_field_holds_a_cell_filter` (around line 751) to match `CellFilter`'s new shape:

```rust
    #[test]
    fn cell_decl_filter_field_holds_a_cell_filter() {
        let span = point(Span::call_site());
        let cell = CellDecl {
            name: "a".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            filter: Some(CellFilter {
                body: cel_parser::Expr::Ident {
                    name: "_".to_string(),
                    span,
                },
                span,
            }),
            require: None,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span,
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert!(matches!(
            &filter.body,
            cel_parser::Expr::Ident { name, .. } if name == "_"
        ));
    }
```

- [ ] **Step 4: Update the top-level grammar doc in `lib.rs`**

In `adam-lang/src/lib.rs`, line 13:

```rust
//! cell_filter        = "filter" expression.
```

and line 25:

```rust
//! requirement        = [ "@" identifier ] expression ";".
```

- [ ] **Step 5: Confirm `adam-lang` fails to compile (expected — Tasks 4/5 fix the parsers)**

Run: `cargo check -p adam-lang --all-targets`
Expected: FAIL, with errors in `ast_parser.rs` and `parser.rs` about missing/extra fields on `CellFilter`/`RequirementDecl` — this confirms the struct change took effect and pinpoints every parser call site Task 4/5 must fix. Do not attempt to fix them here.

- [ ] **Step 6: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
refactor(adam-lang): drop CellFilter's name, make RequirementDecl's optional

Matches the adam_rs API shape from the previous two commits: a filter
carries no name at all; a requirement's name becomes Option<String>,
populated by a new `@identifier` leading marker instead of the old
mandatory `identifier ":"` prefix (ast_parser.rs/parser.rs updated
next).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `adam-lang` — `ast_parser.rs` (CST parser)

**Files:**
- Modify: `adam-lang/src/ast_parser.rs:186` (doc), `:234` (doc), `:284-306` (`parse_cell_filter`), `:503` (doc), `:567-585` (`parse_requirement`)
- Test: `adam-lang/src/ast_parser.rs` (inline test module)

**Interfaces:**
- Consumes: `ast::CellFilter { body, span }`, `ast::RequirementDecl { name: Option<String>, name_span: Option<ExprSpan>, .. }` (Task 3).
- Produces: same `fn parse_cell_filter(&mut self, cursor: &mut TokenCursor, filter_start: proc_macro2::Span) -> Result<ast::CellFilter>` signature (body changes only). Same `fn parse_requirement(&mut self, cursor: &mut TokenCursor) -> Result<ast::RequirementDecl>` signature (body changes only).

- [ ] **Step 1: Update `parse_cell_filter`**

In `adam-lang/src/ast_parser.rs`, replace lines 284-306:

```rust
    /// `cell_filter = "filter" expression.`
    ///
    /// - Precondition: the `filter` keyword has already been consumed by the caller; `filter_start`
    ///   is its span.
    fn parse_cell_filter(
        &mut self,
        cursor: &mut TokenCursor,
        filter_start: proc_macro2::Span,
    ) -> Result<ast::CellFilter> {
        let body = self.parse_cel_expression(cursor)?;
        let body_end = body.span().end;
        Ok(ast::CellFilter {
            body,
            span: ast::ExprSpan {
                start: filter_start,
                end: body_end,
            },
        })
    }
```

- [ ] **Step 2: Update `parse_requirement`**

In `adam-lang/src/ast_parser.rs`, replace lines 567-585:

```rust
    /// `requirement = [ "@" identifier ] expression ";".`
    fn parse_requirement(&mut self, cursor: &mut TokenCursor) -> Result<ast::RequirementDecl> {
        let decl_start = cursor.peek_span();
        let (name, name_span) = if cursor.consume_punct("@") {
            let (name, span) = cursor.consume_ident()?;
            (Some(name), Some(point(span)))
        } else {
            (None, None)
        };
        let body = self.parse_cel_expression(cursor)?;
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::RequirementDecl {
            name,
            name_span,
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }
```

- [ ] **Step 3: Update the three call-site doc comments referencing the old grammar**

In `adam-lang/src/ast_parser.rs`, update the grammar fragment in the doc comments at lines 186, 234, and 503 (the `cell_decl`/`source_decl`/`out_decl` productions) — these already just say `[ cell_filter ]` without inlining its definition, so no text change is needed there; only `parse_cell_filter`'s own doc comment (already rewritten in Step 1) carries the `cell_filter = ...` production.

- [ ] **Step 4: Update tests — filter clauses**

In `adam-lang/src/ast_parser.rs`'s test module:

- `parse_source_with_a_filter` (around line 711): change the source string `"sheet s { source a: i32 = 1 filter clamp: _; }"` to `"sheet s { source a: i32 = 1 filter _; }"`, and delete the line `assert_eq!(filter.name, "clamp");`.
- `parse_cell_with_a_filter` (around line 1405): change `"sheet s { cell a: i32 = 1 filter clamp: _; }"` to `"sheet s { cell a: i32 = 1 filter _; }"`.
- `parse_cell_filter_records_its_name` (around line 1417): **delete this test outright** — a filter no longer has a name to record.
- `parse_cell_with_a_filter_referencing_a_cell` (around line 1428): change `"sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter sum: _ + hi; }"` to `"sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter _ + hi; }"`.
- `recovery_malformed_filter_recovers_at_the_next_sheet_item` (around line 1459): **no change** — its `filter |x: i32|;` fixture is deliberately malformed garbage after the `filter` keyword, unrelated to the old name syntax.

- [ ] **Step 5: Update tests — requirement clauses**

In `adam-lang/src/ast_parser.rs`'s test module, `parse_out_with_requirements_in_declaration_order` (around line 1344): change the source's two requirement lines from

```
max_area: width * height <= max_area;
max_width: width <= max_width;
```

to

```
@max_area width * height <= max_area;
@max_width width <= max_width;
```

and change the two assertions from

```rust
assert_eq!(require.requirements[0].name, "max_area");
assert_eq!(require.requirements[1].name, "max_width");
```

to

```rust
assert_eq!(require.requirements[0].name.as_deref(), Some("max_area"));
assert_eq!(require.requirements[1].name.as_deref(), Some("max_width"));
```

- [ ] **Step 6: Add a new test for the optional-label parse itself**

Add, alongside the existing requirement tests in `adam-lang/src/ast_parser.rs`:

```rust
    #[test]
    fn parse_requirement_without_a_label_leaves_name_none() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
            sheet s {
                out area: f64 := width * height require {
                    width * height <= max_area;
                };
            }
        "#,
            )
            .unwrap();
        let ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        let require = out.require.as_ref().expect("require block present");
        assert_eq!(require.requirements[0].name, None);
    }
```

- [ ] **Step 7: Run `ast_parser.rs`'s tests**

Run: `cargo test -p adam-lang ast_parser::`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add adam-lang/src/ast_parser.rs
git commit -m "$(cat <<'EOF'
refactor(adam-lang): parse the new filter/requirement grammar in ast_parser.rs

cell_filter no longer consumes a leading identifier/colon.
requirement peeks a leading `@` to populate an optional label,
otherwise dispatches straight to expression parsing — unambiguous at
one token of lookahead since `@` is not a valid start of any CEL
expression.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `adam-lang` — `parser.rs` (compile-to-`Sheet` parser)

**Files:**
- Modify: `adam-lang/src/parser.rs:284-320` (`parse_cell_decl`), `:330-410` (`parse_source_decl`), `:412-548` (`parse_cell_filter`), `:1266-1391` (`parse_out_decl`), `:1393-1437` (`parse_requirement`)
- Test: `adam-lang/src/parser.rs` (inline test module)

**Interfaces:**
- Consumes: `adam_rs::Sheet::add_filter(cell, filter)` (Task 1), `adam_rs::Sheet::add_requirement(cell, name: Option<&str>, req)` / `add_out(writer, Vec<(Option<&str>, Requirement)>)` (Task 2).
- Produces: `fn parse_cell_filter(&mut self, ctx: &mut ParseContext, cell_name: &str, cell_span: Span, declared_shape: &TypeShape) -> Result<adam_rs::Filter>` (was `Result<(String, adam_rs::Filter)>`). `fn parse_requirement(&mut self, ctx: &mut ParseContext) -> Result<(Option<String>, Requirement)>` (was `Result<(String, Requirement)>`).

- [ ] **Step 1: Update `parse_cell_filter`**

In `adam-lang/src/parser.rs`, replace the signature and body (lines 412-548):

```rust
    /// `cell_filter = "filter" expression.`
    ///
    /// Builds an [`adam_rs::Filter`] from a single deduced expression: `_` denotes the candidate
    /// value being conformed (of `declared_shape`'s type); every other identifier that names an
    /// already-declared cell is a deduced dependency, exactly as [`Self::parse_deduced_expr`]
    /// resolves them for a `relationship` binding or `out` declaration — see
    /// [`Self::parse_filter_expr`]. `cell_name`/`cell_span`/`declared_shape` describe the
    /// *filtered cell* (for error-message context and the candidate value's type), already
    /// resolved by the caller — [`Self::parse_cell_decl`], [`Self::parse_source_decl`], or
    /// [`Self::parse_out_decl`]. The filtered cell's own `CellId` is not needed
    /// here: the caller attaches the returned `Filter` to it afterwards, via `Sheet::add_filter`.
    ///
    /// # Errors
    /// Returns `Err` if `declared_shape` is a tuple (not yet supported by this builder), if an
    /// identifier inside the expression names neither `_` nor an already-declared cell, if `_`
    /// is never referenced, if the expression's inferred type doesn't match `declared_shape`, or,
    /// if the expression is `RangeInclusive`-typed, if its element type doesn't match
    /// `declared_shape`.
    ///
    /// - Complexity: O(m) in the number of distinct cell identifiers the expression references,
    ///   for this method's own bookkeeping (on top of the expression's own parse/compile cost).
    fn parse_cell_filter(
        &mut self,
        ctx: &mut ParseContext,
        cell_name: &str,
        cell_span: Span,
        declared_shape: &TypeShape,
    ) -> Result<adam_rs::Filter> {
        if matches!(declared_shape, TypeShape::Tuple(_)) {
            return Err(ParseError::new(
                format!("cell `{cell_name}`: filter on a tuple-typed cell is not yet supported"),
                cell_span,
            ));
        }

        let (segment, inputs, underscore_used) = self.parse_filter_expr(ctx, declared_shape)?;

        let value_type_id = cell_type_id(declared_shape);
        let output_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ParseError::new(
                format!("cell `{cell_name}`: filter produced no value"),
                cell_span,
            )
        })?;

        let arg_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let arg_type_ids: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();

        // A `RangeInclusive<T>`-shaped output is checked and built independently of
        // `underscore_used`: a genuine range filter body (`lo..=hi`) essentially never references
        // `_`, so gating this branch on `underscore_used` would make the more specific "range
        // bounds must be" error unreachable for the primary mistake it exists to catch.
        if let Some(range_shape) = self.types.range_entry(output_type_id) {
            if range_shape.element_type_id != value_type_id {
                return Err(ParseError::new(
                    format!(
                        "cell `{cell_name}`: filter range bounds must be `{}`",
                        self.types.display_name(declared_shape)
                    ),
                    cell_span,
                ));
            }

            let default_fn = self
                .types
                .entry_by_type_id(value_type_id)
                .expect("declared cell type registered")
                .default_fn
                .expect("numeric range-filter cell type has a default");
            let placeholder = default_fn();
            let segment = std::rc::Rc::new(RefCell::new(segment));
            let clamp_segment = std::rc::Rc::clone(&segment);
            let bounds_segment = std::rc::Rc::clone(&segment);
            let clamp_fn = range_shape.clamp_fn;
            let bounds_fn = range_shape.bounds_fn;

            return Ok(adam_rs::Filter::range(
                value_type_id,
                arg_ids,
                arg_type_ids,
                move |value, args| clamp_fn(&mut clamp_segment.borrow_mut(), value, args),
                move |args| {
                    bounds_fn(&mut bounds_segment.borrow_mut(), placeholder.as_ref(), args).ok()
                },
            ));
        }

        if !underscore_used {
            return Err(ParseError::new(
                "filter must reference `_` (the value being filtered)",
                cell_span,
            ));
        }

        if output_type_id != value_type_id {
            return Err(ParseError::new(
                format!(
                    "cell `{cell_name}`: filter must produce `{}`",
                    self.types.display_name(declared_shape)
                ),
                cell_span,
            ));
        }

        // `call_dyn_fn` is the same monomorphized-per-registered-type dispatcher `build_method`/
        // `build_match_expr` already use for a deduced expression's scalar output.
        let call_fn = self
            .types
            .entry_by_type_id(value_type_id)
            .expect("declared cell type registered")
            .call_dyn_fn;

        // `RefCell`, not a plain `move` capture: `call_fn` takes `&mut DynSegment`, unlike
        // `DynClosure::call_boxed`'s `&self` the old closure-literal path used.
        let segment = RefCell::new(segment);

        Ok(adam_rs::Filter::new(
            value_type_id,
            arg_ids,
            arg_type_ids,
            move |value, args| {
                let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
                call_args.push(value);
                call_args.extend_from_slice(args);
                call_fn(&mut segment.borrow_mut(), &call_args)
            },
        ))
    }
```

- [ ] **Step 2: Update `parse_cell_decl`'s filter/require handling**

In `adam-lang/src/parser.rs`, in `parse_cell_decl` (around lines 296-319), change the require-block accumulator's element type and the two `add_filter`/`add_requirement` call sites:

```rust
        let require_names_and_reqs: Vec<(Option<String>, Requirement)> = if ctx.is_keyword("require")
        {
            ctx.expect_open_brace()?;
            let mut reqs = Vec::new();
            while !ctx.at_close_brace() {
                reqs.push(self.parse_requirement(ctx)?);
            }
            ctx.expect_close_brace()?;
            reqs
        } else {
            Vec::new()
        };

        ctx.expect_punct(";")?;
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(cell_id, filter)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        for (req_name, requirement) in require_names_and_reqs {
            ctx.sheet
                .add_requirement(cell_id, req_name.as_deref(), requirement)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        Ok(())
```

- [ ] **Step 3: Update `parse_source_decl`'s filter/require handling (mirrors `parse_cell_decl` exactly)**

In `adam-lang/src/parser.rs`, in `parse_source_decl` (around lines 386-409), apply the identical change:

```rust
        let require_names_and_reqs: Vec<(Option<String>, Requirement)> = if ctx.is_keyword("require")
        {
            ctx.expect_open_brace()?;
            let mut reqs = Vec::new();
            while !ctx.at_close_brace() {
                reqs.push(self.parse_requirement(ctx)?);
            }
            ctx.expect_close_brace()?;
            reqs
        } else {
            Vec::new()
        };

        ctx.expect_punct(";")?;
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(cell_id, filter)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        for (req_name, requirement) in require_names_and_reqs {
            ctx.sheet
                .add_requirement(cell_id, req_name.as_deref(), requirement)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        Ok(())
```

- [ ] **Step 4: Update `parse_out_decl`'s filter/require handling**

In `adam-lang/src/parser.rs`, in `parse_out_decl` (around lines 1359-1391), change the requirement-name accumulator and the final `add_out`/`add_filter` calls:

```rust
        let mut requirement_names: Vec<Option<String>> = Vec::new();
        let mut requirements: Vec<Requirement> = Vec::new();
        if ctx.is_keyword("require") {
            ctx.expect_open_brace()?;
            while !ctx.at_close_brace() {
                let (req_name, requirement) = self.parse_requirement(ctx)?;
                requirement_names.push(req_name);
                requirements.push(requirement);
            }
            ctx.expect_close_brace()?;
        }

        ctx.expect_punct(";")?;

        let named_requirements: Vec<(Option<&str>, Requirement)> = requirement_names
            .iter()
            .map(|n| n.as_deref())
            .zip(requirements)
            .collect();

        let out_cell = ctx
            .sheet
            .add_out(writer, named_requirements)
            .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(out_cell, filter)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        ctx.output_names.insert(name, out_cell);

        Ok(())
```

- [ ] **Step 5: Update `parse_requirement`**

In `adam-lang/src/parser.rs`, replace lines 1393-1437:

```rust
    /// `requirement = [ "@" identifier ] expression ";".`
    fn parse_requirement(&mut self, ctx: &mut ParseContext) -> Result<(Option<String>, Requirement)> {
        let name = if ctx.consume_punct("@") {
            let (name, _name_span) = ctx.consume_ident()?;
            Some(name)
        } else {
            None
        };
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        ctx.expect_punct(";")?;

        let bool_type_id = TypeId::of::<bool>();
        let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ctx.err_at(match &name {
                Some(name) => format!("requirement `{name}`: expression produced no value"),
                None => "requirement: expression produced no value".to_string(),
            })
        })?;
        if actual_type_id != bool_type_id {
            let got = self
                .types
                .entry_by_type_id(actual_type_id)
                .map(|e| e.type_name)
                .unwrap_or("?");
            return Err(ctx.err_at(match &name {
                Some(name) => format!("requirement `{name}`: expected `bool`, got `{got}`"),
                None => format!("requirement: expected `bool`, got `{got}`"),
            }));
        }

        let call_fn = self
            .types
            .get("bool")
            .expect("bool always registered")
            .call_dyn_fn;
        let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let input_types: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();
        let segment = RefCell::new(segment);
        let requirement = Requirement::new(input_ids, input_types, move |args| {
            let seg = &mut *segment.borrow_mut();
            let boxed = call_fn(seg, args)?;
            Ok(*boxed
                .downcast::<bool>()
                .expect("checked TypeId::of::<bool>() above"))
        });

        Ok((name, requirement))
    }
```

- [ ] **Step 6: Update tests — filter clauses**

In `adam-lang/src/parser.rs`'s test module, apply `filter <name>: ` → `filter ` (delete the name and colon) in every one of these test sources:

- `cell_filter_with_no_named_dependency_clamps_on_write`: `filter clamp: if _ < 1 { 1 } else if _ > 100 { 100 } else { _ };` → `filter if _ < 1 { 1 } else if _ > 100 { 100 } else { _ };`
- `cell_filter_referencing_a_cell_tracks_its_current_value`: `filter clamp: if _ < 1 { 1 } else if _ > hi { hi } else { _ };` → `filter if _ < 1 { 1 } else if _ > hi { hi } else { _ };`
- `cell_filter_referencing_the_same_value_twice_is_idempotent`: `filter snap: _ - (_ % step);` → `filter _ - (_ % step);`
- `cell_filter_without_underscore_is_a_parse_error`: `filter f: 1;` → `filter 1;`
- `cell_filter_body_type_mismatch_is_a_parse_error`: `filter f: _ > 0;` → `filter _ > 0;`
- `cell_filter_undeclared_identifier_is_a_parse_error`: `filter f: _ + nope;` → `filter _ + nope;`
- `cell_filter_with_a_range_inclusive_body_clamps_on_write`: `filter clamp: 0..=100;` → `filter 0..=100;`
- `cell_filter_range_does_not_require_underscore`: `filter clamp: 0..=100;` → `filter 0..=100;`
- `cell_filter_range_bounds_track_cell_dependencies_live`: `filter clamp: lo..=hi;` → `filter lo..=hi;`
- `cell_filter_range_with_float_cell_type_works`: `filter clamp: 0.0..=100.0;` → `filter 0.0..=100.0;`
- `cell_filter_range_with_mismatched_element_type_is_a_parse_error`: `filter clamp: 0..=100;` → `filter 0..=100;`
- `cell_filter_range_with_mismatched_element_type_and_underscore_is_still_a_range_error`: `filter clamp: (_ as i32)..=100;` → `filter (_ as i32)..=100;`
- `cell_filter_range_returns_none_instead_of_panicking_when_a_bound_expression_fails_to_evaluate`: `filter clamp: 0..=(100 / hi);` → `filter 0..=(100 / hi);`
- `cell_filter_general_expression_still_compiles_to_opaque_kind`: `filter f: if _ < 0 { 0 } else { _ };` → `filter if _ < 0 { 0 } else { _ };`
- `filter_tracks_a_tuple_typed_range_cell_dynamically`: `filter clamp: if _ < a_range.0 { a_range.0 } else if _ > a_range.1 { a_range.1 } else { _ };` → `filter if _ < a_range.0 { a_range.0 } else if _ > a_range.1 { a_range.1 } else { _ };`
- `cell_filter_on_a_tuple_typed_cell_is_a_parse_error`: `filter name: (_.0, _.1);` → `filter (_.0, _.1);`

Rename and rewrite `parse_source_decl_with_a_filter_clause_attaches_a_named_filter` to:

```rust
    #[test]
    fn parse_source_decl_with_a_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { source x: i32 = 5 filter 0..=10; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert!(sheet.filter_kind(x).is_some());
    }
```

Rename and rewrite `parse_out_decl_with_a_filter_clause_attaches_a_named_filter` to:

```rust
    #[test]
    fn parse_out_decl_with_a_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { cell width: i32 = 4; out area := width filter 0..=100; }")
            .unwrap();
        let area = sheet.output_names["area"];
        assert!(sheet.filter_kind(area).is_some());
    }
```

Rename and rewrite `parse_named_filter_attaches_it_under_its_name` to:

```rust
    #[test]
    fn parse_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { cell x: i32 = 0 filter 0..=10; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert!(sheet.filter_kind(x).is_some());
    }
```

Replace `parse_filter_without_a_name_is_a_syntax_error` (its premise is inverted by this change — a filter with no name is now the *only* valid form) with:

```rust
    #[test]
    fn parse_filter_clause_with_a_label_is_now_a_parse_error() {
        let result = parser().parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }");
        assert!(result.is_err());
    }
```

- [ ] **Step 7: Update tests — requirement clauses**

In `adam-lang/src/parser.rs`'s test module:

- `parse_cell_decl_with_a_require_block_attaches_requirements`: `require { positive: x > 0; };` → `require { @positive x > 0; };`
- `parse_source_decl_with_a_require_block_attaches_requirements`: `require { positive: x > 0; };` → `require { @positive x > 0; };`
- `parse_out_with_a_require_block_registers_named_requirements`: `positive: area > 0; small: area < 1000;` → `@positive area > 0; @small area < 1000;`
- `parse_out_require_block_requirement_can_violate`: `too_small: area > 1000;` → `@too_small area > 1000;`
- `parse_requirement_non_bool_body_is_an_error`: `bad: a;` → `@bad a;`
- `parse_out_with_requirements_reports_output_valid_and_violated`: `max_area: width * height <= max_area;` → `@max_area width * height <= max_area;`
- `parse_out_requirement_violation_is_reported_after_propagate`: `max_area: width * height <= max_area;` → `@max_area width * height <= max_area;`
- `parse_out_duplicate_requirement_names_is_error`: `dup: width <= 10.0; dup: width >= 0.0;` → `@dup width <= 10.0; @dup width >= 0.0;`

Add a new test confirming an unlabeled requirement parses and is enforced:

```rust
    #[test]
    fn parse_out_with_an_unlabeled_requirement_is_enforced() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 40.0;
                    cell height: f64 = 30.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }
```

- [ ] **Step 8: Run `parser.rs`'s tests**

Run: `cargo test -p adam-lang parser::`
Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "$(cat <<'EOF'
refactor(adam-lang): parse the new filter/requirement grammar in parser.rs

Mirrors the ast_parser.rs change: cell_filter drops its leading
identifier/colon, requirement gains an optional leading `@identifier`
label. Every call site into adam_rs::Sheet::add_filter/add_requirement/
add_out updated for their new signatures from the previous two
adam-rs commits.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `adam-lang` — `typecheck.rs`

**Files:**
- Modify: `adam-lang/src/typecheck.rs:706-729` (`check_requirements`)
- Test: `adam-lang/src/typecheck.rs` (inline test module)

**Interfaces:**
- Consumes: `ast::RequirementDecl.name: Option<String>` (Task 3).

- [ ] **Step 1: Update `check_requirements`'s diagnostic message**

In `adam-lang/src/typecheck.rs`, replace lines 714-728:

```rust
    for requirement in &require.requirements {
        let (req_ty, req_diags) = check_expr(&requirement.body, resolve);
        diagnostics.extend(req_diags);
        if !req_ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                match &requirement.name {
                    Some(name) => format!(
                        "requirement `{name}` produces `{}`, but requirements must be `bool`",
                        req_ty.name()
                    ),
                    None => format!(
                        "requirement produces `{}`, but requirements must be `bool`",
                        req_ty.name()
                    ),
                },
                requirement.body.span().start,
                requirement.body.span().end,
            ));
        }
    }
```

(`check_filter`, lines 482-537, needs no change — it never references `CellFilter.name`.)

- [ ] **Step 2: Update tests — filter clauses**

In `adam-lang/src/typecheck.rs`'s test module, apply `filter <name>: ` → `filter ` in every one of these test sources:

- `filter_with_matching_types_has_no_diagnostic`: `filter clamp: _;` → `filter _;`
- `filter_with_matching_types_on_a_source_has_no_diagnostic`: `filter clamp: _;` → `filter _;`
- `filter_referencing_a_cell_has_no_diagnostic`: `filter clamp: if _ > hi { hi } else { _ };` → `filter if _ > hi { hi } else { _ };`
- `filter_body_type_mismatch_is_a_diagnostic`: `filter f: _ > 0;` → `filter _ > 0;`
- `filter_without_underscore_is_a_diagnostic`: `filter f: 1;` → `filter 1;`
- `filter_body_type_mismatch_on_an_out_is_a_diagnostic`: `filter f: _ > 0;` → `filter _ > 0;`
- `filter_without_underscore_on_an_out_is_a_diagnostic`: `filter f: 1;` → `filter 1;`
- `filter_body_type_mismatch_on_a_source_is_a_diagnostic`: `filter f: _ > 0;` → `filter _ > 0;`
- `filter_without_underscore_on_a_source_is_a_diagnostic`: `filter f: 1;` → `filter 1;`
- `filter_on_a_tuple_typed_cell_is_a_diagnostic`: `filter f: (_.0, _.1);` → `filter (_.0, _.1);`
- `filter_references_underscore_nested_inside_a_call_has_no_missing_underscore_diagnostic`: `filter f: if true { _ } else { 1 };` → `filter if true { _ } else { 1 };`

- [ ] **Step 3: Update tests — requirement clauses**

In `adam-lang/src/typecheck.rs`'s test module:

- `cell_requirement_non_bool_body_is_a_diagnostic`: `require { positive: x; };` → `require { @positive x; };`
- `requirement_with_bool_body_has_no_diagnostic`: `max_width: width <= max_width;` → `@max_width width <= max_width;`
- `requirement_with_non_bool_body_is_a_diagnostic`: `bogus: width;` → `@bogus width;`

- [ ] **Step 4: Add a test for the unlabeled-requirement diagnostic message**

```rust
    #[test]
    fn unlabeled_requirement_with_non_bool_body_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; \
             out area: f64 := width require { \
                 width; \
             }; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }
```

- [ ] **Step 5: Run `typecheck.rs`'s tests**

Run: `cargo test -p adam-lang typecheck::`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add adam-lang/src/typecheck.rs
git commit -m "$(cat <<'EOF'
fix(adam-lang): drop the requirement name from check_requirements' message when absent

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `adam-lang` — `fmt.rs` (printer)

**Files:**
- Modify: `adam-lang/src/fmt.rs:272-277` (`write_cell`), `:285-297` (`write_requirement`), `:334-339` (`write_out`), `:367-372` (`write_source`)
- Test: `adam-lang/src/fmt.rs` (inline test module)

**Interfaces:**
- Consumes: `ast::CellFilter { body, span }`, `ast::RequirementDecl.name: Option<String>` (Task 3).

- [ ] **Step 1: Update the three filter-emission sites**

In `adam-lang/src/fmt.rs`, in `write_cell` (lines 272-277), `write_out` (lines 334-339), and `write_source` (lines 367-372), each currently:

```rust
    if let Some(filter) = &cell.filter {
        out.push_str(" filter ");
        out.push_str(&filter.name);
        out.push_str(": ");
        out.push_str(&cel_parser::format_expr(&filter.body));
    }
```

becomes:

```rust
    if let Some(filter) = &cell.filter {
        out.push_str(" filter ");
        out.push_str(&cel_parser::format_expr(&filter.body));
    }
```

(substituting `decl.filter` for `cell.filter` in `write_out`/`write_source`, matching each function's existing variable name).

- [ ] **Step 2: Update `write_requirement`**

In `adam-lang/src/fmt.rs`, replace lines 285-297:

```rust
/// Writes one `[ "@" identifier " " ] ...;` requirement.
fn write_requirement(out: &mut String, req: &ast::RequirementDecl, depth: usize) {
    write_trivia(
        out,
        req.blank_line_before,
        req.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    if let Some(name) = &req.name {
        out.push('@');
        out.push_str(name);
        out.push(' ');
    }
    out.push_str(&cel_parser::format_expr(&req.body));
    out.push_str(";\n");
}
```

- [ ] **Step 3: Update tests**

In `adam-lang/src/fmt.rs`'s test module:

- `formats_an_out_with_requirements_in_declaration_order` (source and expected): `max_area: width * height <= max_area;` → `@max_area width * height <= max_area;`
- `formats_a_cell_with_a_require_block`: `positive: x > 0;` → `@positive x > 0;`
- `formats_a_source_with_a_require_block`: `positive: x > 0;` → `@positive x > 0;`
- `formats_a_source_with_a_filter_clause`: `filter clamp: 0..=10;` → `filter 0..=10;`
- `formats_an_out_with_a_filter_clause`: `filter clamp: 0..=100;` → `filter 0..=100;`
- `formats_a_trailing_comment_before_a_requires_closing_brace`: `c: w <= 10.0;` → `@c w <= 10.0;`
- `formats_a_trailing_comment_before_a_cells_requires_closing_brace`: `r: a > 0;` → `@r a > 0;`
- `formats_a_trailing_comment_before_a_sources_requires_closing_brace`: `r: a > 0;` → `@r a > 0;`
- `formats_a_cell_with_a_filter`: `filter clamp: _;` → `filter _;`
- `formats_a_cell_with_a_filter_referencing_a_cell`: `filter clamp: min(_, hi);` → `filter min(_, hi);`
- `format_is_idempotent_through_a_reparse_with_a_filter`: `filter clamp: _;` → `filter _;`

Rename and rewrite `formats_a_named_filter` to:

```rust
    #[test]
    fn formats_a_filter() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell x: i32 = 0 filter 0..=10; }")
            .unwrap();
        assert_eq!(
            format_sheet(&sheet),
            "sheet s {\n    cell x: i32 = 0 filter 0..=10;\n}\n"
        );
    }
```

- [ ] **Step 4: Add a round-trip test for an unlabeled requirement**

```rust
    #[test]
    fn formats_a_requirement_with_no_label() {
        let source = "sheet s {\n    cell x: i32 = 5 require {\n        x > 0;\n    };\n}";
        assert_eq!(format(source), format!("{source}\n"));
    }
```

- [ ] **Step 5: Run `fmt.rs`'s tests**

Run: `cargo test -p adam-lang fmt::`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add adam-lang/src/fmt.rs
git commit -m "$(cat <<'EOF'
fix(adam-lang): print the new filter/requirement grammar in fmt.rs

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `ez-adam` — `clamp_filter`

**Files:**
- Modify: `ez-adam/src/codegen/ast_builder.rs:142-147`

**Interfaces:**
- Consumes: `ast::CellFilter { body, span }` (Task 3).

- [ ] **Step 1: Drop the removed fields from the `CellFilter` literal**

In `ez-adam/src/codegen/ast_builder.rs`, replace lines 142-147:

```rust
    Ok(Some(CellFilter {
        body,
        span: ExprSpan::for_text("_"),
    }))
```

- [ ] **Step 2: Run `ez-adam`'s tests**

Run: `cargo test -p ez-adam`
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add ez-adam/src/codegen/ast_builder.rs
git commit -m "$(cat <<'EOF'
fix(ez-adam): drop CellFilter's removed name/name_span fields in clamp_filter

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `adam-web-ui` — call-site fixes

**Files:**
- Modify: `adam-web-ui/src/labels.rs` (test module, around line 610)
- Modify: `adam-web-ui/src/inspector.rs` (test module, around lines 741-770)

**Interfaces:**
- Consumes: `adam_rs::Sheet::add_filter(cell, filter)` (Task 1).

- [ ] **Step 1: Compiler-driven sweep**

Run:

```bash
cargo check -p adam-web-ui --all-targets
```

For every "this function takes 2 arguments but 3 were supplied" error from `Sheet::add_filter`, delete the middle `"..."` string-literal argument — e.g. `sheet.add_filter(a, "range_0_100", filter)` becomes `sheet.add_filter(a, filter)`. This affects `labels_from_cell_names_populates_range_for_a_range_filtered_cell` in `labels.rs` and `compute_output_status_filter_violated_includes_the_cell_whose_own_filter_failed`/`compute_output_status_filter_violated_empty_when_no_filter_is_violated` (and any other `add_filter` call the compiler flags) in `inspector.rs`.

`adam-web-ui` has no `add_requirement`/`add_out` call sites of its own (it only *reads* `requirement_name`, whose signature is unchanged), so Task 2 requires no changes here.

Repeat `cargo check -p adam-web-ui --all-targets` until it passes with zero errors.

- [ ] **Step 2: Run `adam-web-ui`'s tests**

Run: `cargo test -p adam-web-ui`
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add adam-web-ui/src/labels.rs adam-web-ui/src/inspector.rs
git commit -m "$(cat <<'EOF'
fix(adam-web-ui): drop the name argument from Sheet::add_filter call sites

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: `adam-lang-book` — tests and `.adm2` examples

**Files:**
- Modify: `adam-lang-book/tests/filters.rs:71-84` (`filter_on_an_out_cell`)
- Modify: `adam-lang-book/tests/source.rs:28-44` (`source_with_a_filter`)
- Modify 14 `.adm2` files under `adam-lang-book/book-src/examples/`

**Interfaces:**
- Consumes: the new `.adm2` grammar (Tasks 4/5), `Sheet::filter_name` removed (Task 1).

- [ ] **Step 1: Fix the two tests asserting on the removed `filter_name`**

In `adam-lang-book/tests/filters.rs`, in `filter_on_an_out_cell`, delete the line:

```rust
    assert_eq!(parsed.filter_name(area), Some("clamp"));
```

(the test's remaining lines — `propagate()` then checking the clamped read value — already verify the filter is present and working).

In `adam-lang-book/tests/source.rs`, in `source_with_a_filter`, delete the line:

```rust
    assert_eq!(parsed.filter_name(level), Some("clamp"));
```

- [ ] **Step 2: Update every `.adm2` example with a `filter <name>:` clause**

Apply `filter <name>: ` → `filter ` (delete the name and colon) in each of these files:

- `book-src/examples/filters/filter_on_an_out_cell.adm2`: `out area := width filter clamp: 0..=100;` → `out area := width filter 0..=100;`
- `book-src/examples/filters/range_filter_kind.adm2`: `source level: i32 = 50 filter clamp: 0..=100;` → `source level: i32 = 50 filter 0..=100;`
- `book-src/examples/filters/write_never_filters.adm2`: `source level: i32 = 50 filter clamp: 0..=100;` → `source level: i32 = 50 filter 0..=100;`
- `book-src/examples/filters/raw_value_never_lost.adm2`: `source max: i32 = 100 filter clamp: 0..=200;` → `source max: i32 = 100 filter 0..=200;`, and `source level: i32 = 50 filter clamp: 0..=max;` → `source level: i32 = 50 filter 0..=max;`
- `book-src/examples/filters/derived_cell_diagnosed_not_corrected.adm2`: `cell bound: i32 = 100 filter clamp: 0..=100;` → `cell bound: i32 = 100 filter 0..=100;`
- `book-src/examples/source/source_with_a_filter.adm2`: `source level: i32 = 50 filter clamp: 0..=100;` → `source level: i32 = 50 filter 0..=100;`
- `book-src/examples/tutorial/basic_relationship.adm2`: `cell a: f64 filter clamp: 0.0..=200.0;` → `cell a: f64 filter 0.0..=200.0;`, and `cell b = 2.0 filter clamp: 0.0..=100.0;` → `cell b = 2.0 filter 0.0..=100.0;`
- `book-src/examples/tutorial/constrain.adm2`: `cell a = 5 filter clamp: 1..=100;` → `cell a = 5 filter 1..=100;`, and `cell b = 10 filter clamp: 1..=100;` → `cell b = 10 filter 1..=100;`
- `book-src/examples/outputs/basic_output.adm2`: `source width = 10 filter clamp: 0..=100;` → `source width = 10 filter 0..=100;`, and `source height = 20 filter clamp: 0..=100;` → `source height = 20 filter 0..=100;`
- `book-src/examples/tutorial/basic_output.adm2`: same two edits as `outputs/basic_output.adm2`
- `book-src/examples/tutorial/clamp_demo.adm2`: `source level: i32 = 50 filter clamp: 0..=100;` → `source level: i32 = 50 filter 0..=100;`
- `book-src/examples/tutorial/inequality.adm2`: `cell a = 10 filter clamp: 0..=100;` → `cell a = 10 filter 0..=100;`, `cell b = 20 filter clamp: 0..=100;` → `cell b = 20 filter 0..=100;`, `cell c = 30 filter clamp: 0..=100;` → `cell c = 30 filter 0..=100;`

- [ ] **Step 3: Update every `.adm2` example with a labeled requirement (preserve the label as `@name`)**

- `book-src/examples/tutorial/area_with_requirement.adm2`: `not_too_big: area <= 300;` → `@not_too_big area <= 300;`
- `book-src/examples/outputs/requirement_diagnostic.adm2`: `not_too_big: area <= 300;` → `@not_too_big area <= 300;`
- `book-src/examples/outputs/multiple_requirements.adm2`: `not_negative: clamped >= 0;` → `@not_negative clamped >= 0;`, and `not_too_big: clamped <= 100;` → `@not_too_big clamped <= 100;`
- `book-src/examples/source/source_with_a_requirement.adm2`: `positive: width > 0;` → `@positive width > 0;`

- [ ] **Step 4: Run `adam-lang-book`'s tests**

Run: `cargo test -p adam-lang-book`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add adam-lang-book/tests/filters.rs adam-lang-book/tests/source.rs adam-lang-book/book-src/examples
git commit -m "$(cat <<'EOF'
fix(adam-lang-book): migrate bundled .adm2 examples to the new filter/requirement grammar

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: `begin` — `.adm2` examples

**Files:**
- Modify: `begin/examples/image_resize.adm2:111-113`
- Modify: `begin/examples/inequality.adm2:2-5`
- Modify: `begin/examples/out-cell.adm2:16-18`

**Interfaces:**
- Consumes: the new `.adm2` grammar (Tasks 4/5).

- [ ] **Step 1: Update `image_resize.adm2`'s requirement labels**

In `begin/examples/image_resize.adm2`, lines 112-113:

```
        @width_max width_pixels <= 300000;
        @height_max height_pixels <= 300000;
```

- [ ] **Step 2: Update `inequality.adm2`'s filter clauses**

In `begin/examples/inequality.adm2`, lines 2-5:

```
    cell max_v = 100 filter 0..=200;
    cell a = 0 filter 0..=max_v;
    cell b = 42 filter 0..=max_v;
    cell c = 100 filter 0..=max_v;
```

- [ ] **Step 3: Update `out-cell.adm2`'s requirement label**

In `begin/examples/out-cell.adm2`, line 17:

```
        @min_a a <= b;
```

- [ ] **Step 4: Rebuild `begin` and confirm every bundled example still parses**

Run: `cargo build -p begin --no-default-features`
Expected: succeeds (this exercises the bundled `.adm2` files' parse path at either build or first-run time — confirm by checking `begin`'s own test suite, which loads these examples).

Run: `cargo test -p begin --no-default-features`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add begin/examples/image_resize.adm2 begin/examples/inequality.adm2 begin/examples/out-cell.adm2
git commit -m "$(cat <<'EOF'
fix(begin): migrate bundled .adm2 examples to the new filter/requirement grammar

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: `adam-lang-book` — prose and grammar reference

**Files:**
- Modify: `adam-lang-book/book-src/filters.md:1-27`
- Modify: `adam-lang-book/book-src/reference.md:15-47`, `:153-160`, `:181-187`, `:216`
- Modify: `adam-lang-book/book-src/outputs.md:6-9`, `:57-59`

**Interfaces:**
- Consumes: the grammar from §2 of the design spec.

- [ ] **Step 1: Update `filters.md`'s grammar block and label prose**

In `adam-lang-book/book-src/filters.md`, replace lines 5-23:

```text
cell_filter = "filter" expression.
```

A `filter` clause is optional and trails a `cell`, [`source`](source.md), or
[`out`](outputs.md) declaration's type/initializer. Its `expression` is
[deduced](expressions.md#deduced-dependencies) exactly like a relationship binding's, plus
one reserved identifier: `_` always refers to the *candidate value being conformed* (of the
filtered cell's own declared type), never a cell. `_` is reserved inside a filter expression
only; outside one it's an ordinary identifier (or the [conditional](conditionals.md)
default-branch token). A filter carries no name — a cell can have at most one, so nothing
ever needs to distinguish "which filter."

```adam
cell level: i32 = 50 filter 0..=100;             // a fixed range
cell level: i32 = 50 filter 0..=max;             // upper bound is another cell
cell level: i32 = 50 filter clamp(_, 0, max);    // an arbitrary expression over `_`
```

- [ ] **Step 2: Update the "filter on an output cell"/"a source cell can be filtered too" cross-reference prose**

In `adam-lang-book/book-src/filters.md`, line 102 references `cell_filter` by name in a sentence about `out`'s grammar — no wording change is needed there (it still correctly says an `out` carries the same optional `cell_filter` clause); only the grammar block itself (Step 1) and any other literal `filter <name>:` example text need updating. Search the rest of `filters.md` for any remaining `filter <name>:` example snippets outside the block already fixed in Step 1, and apply the same `filter <name>: ` → `filter ` edit if found.

- [ ] **Step 3: Update `reference.md`'s grammar block**

In `adam-lang-book/book-src/reference.md`, replace lines 15-33:

```text
cell_decl          = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
cell_type_init     = (":" type_expr ["=" expression]) | ("=" expression).
cell_filter        = "filter" expression.
source_decl        = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".

type_expr          = identifier
                    | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".

relationship_decl  = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".

conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
conditional_branch = (expression | "_") "=>" "{" { relationship_decl } "}" [ "," ].

out_decl           = "out" identifier [ ":" type_expr ] ":=" expression
                       [ cell_filter ] [ require_block ] ";".
require_block      = "require" "{" { requirement } "}".
requirement        = [ "@" identifier ] expression ";".
```

- [ ] **Step 4: Update `reference.md`'s "Filters" bullet list**

In `adam-lang-book/book-src/reference.md`, replace lines 153-160:

```markdown
- `cell_filter = "filter" expression`, trailing a `cell_decl`, `source_decl`, or `out_decl` —
  a filter attaches to any cell kind, with no per-kind restriction (see
  [a source cell can be filtered too](source.md#a-source-cell-can-be-filtered-too)). A filter
  carries no name; `_` inside the expression denotes the candidate value (of the cell's own
  declared type); every other identifier is a deduced dependency. The expression must
  reference `_` at least once (unless it's a range expression, `lo..=hi`, which is exempt)
  and must produce the filtered cell's own type.
```

- [ ] **Step 5: Update `reference.md`'s "Outputs and requirements" bullet and error table**

In `adam-lang-book/book-src/reference.md`, replace line 181:

```markdown
- `require { [ "@" name ] expr; ... }` attaches boolean checks, each with an optional
  `@name` label. Unlike `filter`, `require` is not tied to `out`: a `require` block may
  trail a `cell`, `source`, or `out` declaration's initializer, with the same meaning in
  every case. Each `requirement`'s own dependencies are deduced separately from its
  declaration's own expression. A failing requirement never stops the sheet from resolving,
  or its cell's own value from being computed: it's reported as a diagnostic, nothing more,
  queryable by a host (see [the host embedding API](#the-host-embedding-api)).
```

Update the error-message table row at line 216:

```markdown
| `requirement [\`name\`: ]expected \`bool\`, got \`T\`` | a `require`ment body that isn't boolean — the `` `name`: `` segment appears only when the requirement is labeled |
```

- [ ] **Step 6: Update `outputs.md`'s grammar block and label prose**

In `adam-lang-book/book-src/outputs.md`, replace line 8:

```text
requirement = [ "@" identifier ] expression ";".
```

Replace lines 57-59:

```markdown
A failed requirement never stops the sheet from resolving, and never stops `area` from being
computed and readable; a host can query which requirements are currently failing precisely
because nothing else in the sheet notices a requirement failing on its own (see the
[host embedding API](reference.md#the-host-embedding-api)). A requirement's `@name` is
optional — present only when an application wants to report *which* requirement failed; it
happens to read naturally when it echoes a cell name (`@not_too_big`, `@width_max`), but it
isn't a cell reference and doesn't have to match one.
```

- [ ] **Step 7: Rebuild the book and confirm it still renders**

Run: `cargo test -p adam-lang-book`
Expected: all pass (the `{{#include}}` macros pull the already-fixed `.adm2` files from Task 10, so any remaining old-syntax fragment left in prose would only be visually stale, not test-breaking — the grep in Step 8 catches those).

- [ ] **Step 8: Confirm no stale grammar text remains**

Run:

```bash
grep -rn 'filter [a-z_]*:' adam-lang-book/book-src/*.md
grep -rn 'identifier ":" expression' adam-lang-book/book-src/*.md
```

Expected: no output from either command.

- [ ] **Step 9: Commit**

```bash
git add adam-lang-book/book-src/filters.md adam-lang-book/book-src/reference.md adam-lang-book/book-src/outputs.md
git commit -m "$(cat <<'EOF'
docs(adam-lang-book): document the new filter/requirement grammar

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Full-workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Confirm the VS Code syntax grammar and `adam-lsp` need no changes**

Already verified during planning: `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json` highlights `filter`/`require` as bare keywords (`"\\b(sheet|cell|source|relationship|conditional|out|require|filter)\\b"`) with no label-specific pattern, and `adam-lsp` contains no text matching the old grammar productions. No action needed; this step is a recorded confirmation, not a code change.

- [ ] **Step 2: Format**

Run: `cargo fmt --all`

- [ ] **Step 3: Build the whole workspace with zero warnings**

Run: `cargo build --workspace`
Expected: succeeds, zero warnings (per this repo's `CLAUDE.md`, a plain build warning — e.g. an unused `mut` — is not caught by clippy's `-D warnings` and must be fixed here).

- [ ] **Step 4: Test the whole workspace with zero warnings**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: both succeed, zero warnings, all tests pass.

- [ ] **Step 5: Lint per this repo's three-invocation convention**

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all three succeed with zero warnings.

- [ ] **Step 6: Docs**

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`
Expected: succeeds, zero warnings (catches any stale intra-doc link left by the `CellFilter`/`RequirementDecl` field removals, e.g. a doc comment elsewhere linking to `CellFilter::name`).

- [ ] **Step 7: Final grep sweep for any remaining old-syntax fixture across the whole workspace**

```bash
grep -rn 'filter [a-z_][a-z_]*:' --include=*.adm2 --include=*.rs .
```

Expected: no output. (This is a final safety net, not a substitute for the per-task greps already run — any hit here means a task above missed a call site.)

- [ ] **Step 8: Update the spec's status**

In `docs/superpowers/specs/2026-09-09-filter-require-labels-design.md`, change the header's `**Status:** Approved (design), not yet implemented` to `**Status:** Implemented`.

- [ ] **Step 9: Commit**

```bash
git add docs/superpowers/specs/2026-09-09-filter-require-labels-design.md
git commit -m "$(cat <<'EOF'
docs: mark filter/requirement label cleanup spec as implemented

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
