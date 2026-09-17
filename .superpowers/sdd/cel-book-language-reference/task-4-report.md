# Task 4 report — optional standard library chapter

## Inputs read

- `task-4-brief.md`
- `task-1-report.md`
- current `cel-lang-book` chapters

## Assigned files updated

- `cel-lang-book/book-src/standard-library.md`
- `cel-lang-book/book-src/SUMMARY.md`
- `cel-lang-book/book-src/intro.md`
- `cel-lang-book/book-src/reference.md`
- `cel-lang-book/tests/examples.rs`
- `cel-lang-book/Cargo.toml`

## Additional generated artifact

- `Cargo.lock` changed as required by the new `cel-std` dev-dependency.
- This report was written to `task-4-report.md` and left unstaged.

## What changed

### `standard-library.md`
- Added a new tutorial/reference chapter for numeric helper calls.
- Explained that `round` belongs to the core environment while the remaining
  functions come from an optional library layered over the core expression
  language.
- Stated that optional-library functions are available only when the evaluation
  environment installs that library.
- Documented exactly these functions:
  - `round`
  - `min`
  - `max`
  - `clamp`
  - `abs`
  - `signum`
  - `sqrt`
  - `floor`
  - `ceil`
  - `trunc`
- Included supported numeric domains and result-type rules:
  - `round`: `f64 -> f64`
  - `min`/`max`/`clamp`: same-type numeric operands over all 12 integer types
    plus `f32` and `f64`, returning that same type
  - `abs`/`signum`: signed integers plus `f32`/`f64`, returning the same type
  - `sqrt`/`floor`/`ceil`/`trunc`: `f32` and `f64`, returning the same type
- Explicitly stated the category split:
  - no documented function is integer-only
  - `round`, `sqrt`, `floor`, `ceil`, and `trunc` are float-only
  - `min`, `max`, and `clamp` support both integers and floats
  - `abs` and `signum` support signed integers and floats, but not unsigned
    integers
- Recorded the confirmed semantics from the inventory:
  - `min`/`max` require same-type operands
  - `clamp` requires same-type operands and ordered bounds
  - `abs` reports `arithmetic overflow` for the minimum signed integer
  - `signum` supports signed integers and floats
  - `sqrt`/`floor`/`ceil`/`trunc` support floats
  - successful calls are otherwise infallible where confirmed
- Kept examples as CEL-only fenced `text` blocks.

### `SUMMARY.md`
- Added **Standard library** after **Operators** and before **Control flow**.
- Preserved **Lexical conventions** immediately after **Introduction**.

### `intro.md`
- Updated the opening scope sentence to include numeric helper calls.
- Added the new chapter to the ordered tutorial list.
- Added a concise note explaining that the standard-library chapter distinguishes
  the core `round` call from optional environment-installed library functions.
- Clarified that environment-dependent availability is called out explicitly in
  prose.

### `reference.md`
- Added a compact **Numeric call table** instead of duplicating the full new
  chapter.
- Cross-linked to `standard-library.md` for worked examples and fuller prose.
- Summarized supported operands, result types, and error notes for
  `round`, `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`,
  and `trunc`.
- Added a short bullet summary of which functions are float-only, support both
  integers and floats, or support signed integers plus floats.

### `tests/examples.rs`
- Added checked examples for:
  - core `round`
  - optional-library `min`, `max`, and `clamp`
  - optional-library `abs` and `signum`
  - optional-library `sqrt`, `floor`, `ceil`, and `trunc`
- Kept these as Rust test infrastructure only, not book prose.

### `Cargo.toml`
- Added `cel-std` as a `dev-dependency` so `cel-lang-book` integration tests can
  exercise the optional library.

## Validation

### `cargo test -p cel-lang-book`

Result:

```text
Compiling cel-std v0.1.0 (D:\repos\github.com\stlab\cel-rs\.claude\worktrees\cel-lang-book\improvements\cel-std)
Compiling cel-lang-book v0.1.0 (D:\repos\github.com\stlab\cel-rs\.claude\worktrees\cel-lang-book\improvements\cel-lang-book)
Finished `test` profile [unoptimized + debuginfo] target(s) in 2.30s
Running tests\examples.rs ...
test result: ok. 10 passed; 0 failed
Doc-tests cel_lang_book
test result: ok. 0 passed; 0 failed
```

Exit code: `0`

### `mdbook build cel-lang-book`

Result:

```text
INFO Book building has started
INFO Running the html backend
INFO HTML book written to `D:\repos\github.com\stlab\cel-rs\.claude\worktrees\cel-lang-book\improvements\cel-lang-book\book-dist`
```

Exit code: `0`

## Scope control

- Preserved unrelated pre-existing worktree edits.
- Updated only the assigned book/test files plus the necessary lockfile and this
  unstaged report artifact.
- Avoided user-facing Rust setup details in book prose.

## Task 4 review fixes

### Findings addressed

1. `cel-lang-book/book-src/standard-library.md`
   - Replaced evaluator-facing wording in the `abs` and `signum` bullets with
     pure language behavior:
     - `abs` now says it succeeds for every supported operand except the minimum
       signed integer value.
     - `signum` now says it succeeds for every supported operand.
2. `cel-lang-book/book-src/reference.md`
   - Replaced the imprecise `sqrt` note with the exact rule that inputs below
     zero yield `NaN` rather than an error.

### Covering validation

Command:

```text
cargo test -p cel-lang-book
```

Output:

```text
Finished `test` profile [unoptimized + debuginfo] target(s) in 0.47s
Running unittests src\lib.rs (target\debug\deps\cel_lang_book-c692a1e814ea0cbf.exe)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

Running tests\examples.rs (target\debug\deps\examples-226b5de2fa96d5db.exe)

running 10 tests
test array_literal_round_trips_through_dynamic_array ... ok
test heterogeneous_array_is_rejected ... ok
test closure_literal_compiles_and_calls ... ok
test arithmetic_respects_precedence ... ok
test optional_standard_library_supports_float_projection_examples ... ok
test if_expression_selects_the_true_branch ... ok
test op_lookup_resolves_identifiers_from_a_custom_scope ... ok
test optional_standard_library_supports_abs_and_signum_examples ... ok
test round_is_available_in_the_core_environment ... ok
test optional_standard_library_supports_min_max_and_clamp_examples ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

Doc-tests cel_lang_book

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Exit code: `0`

Command:

```text
mdbook build cel-lang-book
```

Output:

```text
INFO Book building has started
INFO Running the html backend
INFO HTML book written to `D:\repos\github.com\stlab\cel-rs\.claude\worktrees\cel-lang-book\improvements\cel-lang-book\book-dist`
```

Exit code: `0`
