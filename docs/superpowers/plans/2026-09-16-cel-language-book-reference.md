# CEL Language Book Reference and Standard Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the CEL book into a precise K&R-style tutorial and reference for the implemented language, including its Rust-derived lexical forms, built-in types, numeric literal suffixes, and optional `cel-std` library.

**Architecture:** Keep the book static and language-focused. Derive every documented token, type, operator, literal, and standard-library function from `cel-parser`, `cel-runtime`, `cel-std`, and their tests/examples; present implementation-dependent facts as language rules without exposing Rust API machinery. Place lexical conventions immediately after the introduction, add a dedicated optional standard-library chapter, and use the reference chapter for compact grammar and type tables.

**Tech Stack:** mdBook, Markdown, Rust source/tests as the behavioral source of truth, `mdbook build`, `cargo test -p cel-lang-book`.

**Spec:** `docs/superpowers/specs/2026-09-16-cel-language-book-design.md`

## Global Constraints

- The book documents CEL syntax and semantics, not Rust APIs, parser internals, repository layout, or diagnostics.
- Every language claim must be supported by current implementation behavior or an existing test/example.
- Rust-style numeric suffixes are part of the documented CEL literal syntax: unsuffixed integers are `i32`, unsuffixed floats are `f64`, and accepted explicit suffixes are documented exactly.
- Built-in scalar types are documented exactly as implemented: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64`, `bool`, and `String`.
- `cel-std` is optional; its functions are available only when the optional library is installed.
- CEL examples use fenced `text` blocks or CEL-language snippets, never Rust API examples.
- Validate with `mdbook build cel-lang-book` and `cargo test -p cel-lang-book`.

---

### Task 1: Build the language inventory from implementation and tests

**Files:**
- Read: `cel-parser/src/lex_lexer.rs`
- Read: `cel-parser/src/lib.rs`
- Read: `cel-parser/src/ast.rs`
- Read: `cel-parser/src/op_table.rs`
- Read: `cel-parser/src/ty.rs`
- Read: `cel-std/src/lib.rs`
- Read: `cel-std/src/math.rs`
- Read: `cel-lang-book/tests/examples.rs`
- Read: `cel-parser` and `cel-std` test modules
- Modify: `docs/superpowers/plans/2026-09-16-cel-language-book-reference.md` only if the inventory reveals a required scope correction

**Interfaces:**
- Consumes: parser token handling, literal conversion, built-in scalar names, operator registrations, casts, and standard-library registrations.
- Produces: an implementation-backed checklist used by Tasks 2–5; no committed runtime code.

- [ ] **Step 1: Enumerate lexical token categories**

Record the exact forms accepted by `LexLexer`: identifiers, boolean literals, integer and float literals, strings, chars, bytes, byte strings, C strings, delimiter groups, punctuation, compound operators, and comments/trivia behavior.

- [ ] **Step 2: Enumerate literal conversions and suffixes**

Use `push_literal_token` and its tests to record:

```text
Integers: unsuffixed/i32, i8, i16, i64, i128, isize,
          u8, u16, u32, u64, u128, usize
Floats:   unsuffixed/f64, f32
Other:    true, false, strings, chars, bytes, byte strings, C strings
```

Confirm whether each form belongs in the public language chapter or only in the reference’s implementation-defined literal table.

- [ ] **Step 3: Enumerate built-in scalar types and casts**

Cross-check `Ty`, `builtin_scalar_type`, and cast registrations so the type chapter and cast chapter use the same names and do not omit a supported primitive.

- [ ] **Step 4: Enumerate operators and operand rules**

Extract the operator names, arities, homogeneous/heterogeneous operand types, string operations, boolean operations, shift rules, range forms, tuple indexing, and overflow/error semantics from `op_table.rs` and tests.

- [ ] **Step 5: Enumerate optional standard-library functions**

Record the signatures and supported numeric types for:

```text
min(x, y)
max(x, y)
clamp(x, lower, upper)
abs(x)
signum(x)
sqrt(x)
floor(x)
ceil(x)
trunc(x)
round(x)
```

Confirm `round` from `cel-parser` and the remaining functions from `cel-std`; record `abs` overflow and `clamp` bound errors exactly.

---

### Task 2: Rewrite lexical conventions and literal/type chapters

**Files:**
- Modify: `cel-lang-book/book-src/lexical-conventions.md`
- Modify: `cel-lang-book/book-src/literals-and-types.md`
- Modify: `cel-lang-book/book-src/SUMMARY.md`
- Modify: `cel-lang-book/book-src/intro.md`

**Interfaces:**
- Consumes: Task 1’s exact token and type inventory.
- Produces: an early tutorial chapter and a complete literal/type chapter that later chapters can link to.

- [ ] **Step 1: Move lexical conventions immediately after the introduction**

Update `book-src/SUMMARY.md` and the chapter list in `intro.md` so lexical conventions is the first tutorial chapter after Introduction.

- [ ] **Step 2: Write the lexical chapter as a token reference**

Organize it as:

1. source text, whitespace, and comments;
2. identifiers and reserved words;
3. delimiters and punctuation;
4. operator tokens, including compound operators;
5. literal token categories;
6. how token sequences become expressions.

Use tables and small CEL snippets. Link the lexical background to the Rust Reference, but state CEL’s accepted forms directly.

- [ ] **Step 3: Rewrite literals and types around the complete type table**

Document all implemented scalar types, their literal suffixes, defaults, and representative values. Include strings, chars, bytes, byte strings, C strings, booleans, unit, tuples, arrays, ranges, and closures with clear distinctions between scalar and compound values.

- [ ] **Step 4: Remove vague and negative framing**

Replace phrases such as “where applicable,” “when the language permits,” “not every dialect,” and host-implementation commentary with direct statements of accepted syntax and links to `reference.md`.

- [ ] **Step 5: Review the result against the inventory**

Check every suffix, type name, punctuation form, and token example against the source inventory before moving on.

---

### Task 3: Improve the tutorial chapters and reference manual

**Files:**
- Modify: `cel-lang-book/book-src/expressions.md`
- Modify: `cel-lang-book/book-src/operators.md`
- Modify: `cel-lang-book/book-src/control-flow.md`
- Modify: `cel-lang-book/book-src/collections.md`
- Modify: `cel-lang-book/book-src/casts-and-closures.md`
- Modify: `cel-lang-book/book-src/reference.md`

**Interfaces:**
- Consumes: Task 1’s operator/type inventory and Task 2’s terminology.
- Produces: consistent tutorial chapters and a compact reference chapter with no API examples.

- [ ] **Step 1: Establish a consistent chapter pattern**

For each tutorial chapter, use the sequence: concept, syntax, worked CEL examples, rules, edge cases, reference link. Keep examples executable as language snippets and avoid Rust setup code.

- [ ] **Step 2: Clarify expressions and calls**

Document identifiers, function calls, member-style tuple indexing, grouping, tuples, arrays, conditionals, ranges, and closures as expression forms. Explain scope in language terms rather than `OpLookup` terms.

- [ ] **Step 3: Clarify operator semantics**

Add a precedence table, associativity notes, operand type table, string concatenation behavior, short-circuiting, range parsing, overflow behavior, and shift-count rules where confirmed by tests.

- [ ] **Step 4: Clarify control flow, collections, casts, and closures**

State exact branch and range forms, homogeneous-array rules, tuple/array distinctions, supported cast targets and sources, closure parameter syntax, closure scope behavior, and the documented error cases.

- [ ] **Step 5: Make the reference manual authoritative**

Update grammar, token/type tables, literal suffix tables, operator tables, standard built-ins, collection rules, and closure grammar. Keep implementation-specific names out of the prose while retaining precise language behavior.

- [ ] **Step 6: Sweep for prohibited writing patterns**

Search all `book-src/*.md` for repository/API/implementation references and negative framing. Rewrite each occurrence as a positive language rule or link to the relevant reference section.

---

### Task 4: Add the optional CEL standard-library chapter

**Files:**
- Create: `cel-lang-book/book-src/standard-library.md`
- Modify: `cel-lang-book/book-src/SUMMARY.md`
- Modify: `cel-lang-book/book-src/intro.md`
- Modify: `cel-lang-book/book-src/reference.md`
- Modify: `cel-lang-book/Cargo.toml` if checked examples require the optional `cel-std` dependency
- Modify: `cel-lang-book/tests/examples.rs` if checked examples are needed for the documented functions

**Interfaces:**
- Consumes: `cel-parser` built-in `round` behavior and `cel-std`’s `install`/math functions.
- Produces: a reader-facing chapter describing the optional function library without implying that every CEL environment provides it.

- [ ] **Step 1: Add the chapter to the book navigation**

Place “Standard library” after Operators and before Control flow, because standard functions extend expression examples without changing core grammar.

- [ ] **Step 2: Explain optional availability**

State that the functions in this chapter are an optional library layered on top of the core language. Core CEL syntax remains usable without them; expressions using these names require the library to be available in the evaluation environment.

- [ ] **Step 3: Document function signatures and domains**

Use a compact table for `round`, `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`, and `trunc`, including accepted numeric types and result types.

- [ ] **Step 4: Document semantic edge cases**

Include CEL examples for:

```text
min(3, 5)
max(3.0, 5.0)
clamp(12, 0, 10)
abs(-7)
sqrt(9.0)
floor(3.8)
ceil(3.2)
trunc(-3.8)
signum(-4)
round(3.5)
```

Describe `abs` overflow, invalid clamp bounds, float `NaN` behavior where tests establish it, and the distinction between integer-only and float-only functions.

- [ ] **Step 5: Add focused checked examples**

Extend `tests/examples.rs` only where the existing public parser API can check the standard library without exposing Rust setup in the book. Keep the Rust test code as test infrastructure; keep the chapter examples in CEL.

---

### Task 5: Validate, reconcile, and finalize the book

**Files:**
- Modify: any `cel-lang-book/book-src/*.md` files requiring corrections from validation
- Modify: `cel-lang-book/tests/examples.rs` only for failing or missing representative checks

**Interfaces:**
- Consumes: all previous tasks.
- Produces: a built, internally linked, implementation-accurate CEL book.

- [ ] **Step 1: Run the book test suite**

Run:

```text
cargo test -p cel-lang-book
```

Expected: all checked examples pass.

- [ ] **Step 2: Build the book**

Run:

```text
mdbook build cel-lang-book
```

Expected: `cel-lang-book/book-dist/index.html` is generated with no broken Markdown or link errors.

- [ ] **Step 3: Audit navigation and cross-links**

Verify that `SUMMARY.md` order matches the introduction, the standard-library chapter is reachable, and every relative link to `reference.md` and other chapters resolves.

- [ ] **Step 4: Perform the final language-content sweep**

Search for `cel-parser`, `cel-runtime`, `OpLookup`, `CELParser`, `call0`, repository URLs, implementation references, “not every,” “where applicable,” and “when the language permits.” Each remaining match must either be removed, rewritten as a language rule, or be a deliberate external reference such as the Rust Reference link.

- [ ] **Step 5: Review rendered output**

Inspect the generated pages for readable tables, short examples, consistent terminology, and a natural tutorial progression from tokens to values to expressions, operators, control flow, collections, closures, and optional library functions.

- [ ] **Step 6: Commit the completed documentation pass**

```text
git add cel-lang-book docs/superpowers/plans/2026-09-16-cel-language-book-reference.md
git commit -m "docs: complete CEL language reference and standard library"
```
