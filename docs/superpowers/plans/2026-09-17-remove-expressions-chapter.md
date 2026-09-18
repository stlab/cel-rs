# Removing Redundant Expressions Chapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans (inline execution) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the redundant Expressions chapter while preserving and relocating its unique guidance into the appropriate chapters.

**Architecture:** Treat CEL's expression model as introductory context, place postfix syntax and precedence with operators, and place tuple/indexing distinctions with collections. Remove the chapter from the mdBook source and regenerate the checked-in HTML output.

**Tech Stack:** Markdown, mdBook, ripgrep.

**Spec:** User request to review `cel-lang-book/book-src/expressions.md`, move unique information, and eliminate the chapter.

## Global Constraints

- Preserve all documented language behavior.
- Keep the book's existing chapter structure and prose style.
- Rebuild `cel-lang-book/book-dist/` after source changes.
- Do not retain links or navigation entries to `expressions.md`.

---

### Task 1: Relocate unique Expressions content

**Files:**
- Modify: `cel-lang-book/book-src/intro.md`
- Modify: `cel-lang-book/book-src/literals-and-types.md`
- Modify: `cel-lang-book/book-src/operators.md`
- Modify: `cel-lang-book/book-src/collections.md`

**Interfaces:**
- Consumes: Unique rules currently documented in `cel-lang-book/book-src/expressions.md`.
- Produces: Equivalent guidance in the remaining tutorial chapters.

- [ ] **Step 1: Add the general expression model to the introduction.**

Add a concise paragraph after the introduction's opening paragraph explaining that every CEL form produces a value or composes a value, and that identifiers, calls, operators, collections, control flow, casts, and closures are expression forms.

- [ ] **Step 2: Add call syntax to the expressions-bearing chapters.**

Document zero-argument and comma-separated call syntax in `operators.md`, alongside postfix operators, including that call argument lists do not accept trailing commas.

- [ ] **Step 3: Add tuple indexing and indexing exclusions to collections.**

Document `.N` tuple indexing, its unsuffixed-integer requirement, the absence of member access and bracket indexing, and the distinction between tuple indexing and arrays.

- [ ] **Step 4: Add any missing grouping/tuple nesting statement to literals and types.**

Ensure the literals chapter states that grouping, tuples, arrays, ranges, and closures are expression forms that can nest in one another where the existing grammar permits.

### Task 2: Remove the redundant chapter and references

**Files:**
- Modify: `cel-lang-book/book-src/SUMMARY.md`
- Modify: `cel-lang-book/book-src/intro.md`
- Modify: `cel-lang-book/book-src/standard-library.md`
- Delete: `cel-lang-book/book-src/expressions.md`

**Interfaces:**
- Consumes: Relocated content from Task 1.
- Produces: A navigation tree with no Expressions chapter or dead links.

- [ ] **Step 1: Remove the Expressions entry from the summary and introduction ordering.**

- [ ] **Step 2: Replace remaining cross-references to the deleted chapter with direct wording or references to Operators, Collections, or the Reference Manual.**

- [ ] **Step 3: Delete `expressions.md`.**

### Task 3: Regenerate and verify the book

**Files:**
- Regenerate: `cel-lang-book/book-dist/`

- [ ] **Step 1: Run `mdbook build cel-lang-book`.**

- [ ] **Step 2: Search the source and generated output for `expressions.md`, `Expressions`, and dead links.**

- [ ] **Step 3: Review the diff and confirm the generated navigation no longer includes the deleted chapter.**
