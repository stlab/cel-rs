# annotate-snippets Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-rolled rustc-style diagnostic formatter in `CELError::format_rustc_style` with the `annotate-snippets` crate and remove the `owo-colors` dependency.

**Architecture:** Add a private `line_col_to_byte_offset` helper that converts `proc_macro2` line/column positions to byte offsets (required by annotate-snippets). Replace the manual ANSI string-building in `format_rustc_style` with a `Group`/`Snippet` builder chain fed to `renderer.render()`. Add a `renderer: &Renderer` parameter so callers choose `Renderer::plain()` or `Renderer::styled()`.

**Tech Stack:** Rust, `annotate-snippets 0.12`, `proc-macro2`

**Spec:** `docs/superpowers/specs/2026-06-03-annotate-snippets-design.md`

---

## File Map

| File | Change |
|------|--------|
| `cel-runtime/Cargo.toml` | Add `annotate-snippets`, remove `owo-colors` |
| `cel-runtime/src/parser/error.rs` | Add helper, replace `format_rustc_style` body, update imports |
| `cel-runtime/src/parser/mod.rs` | Update call sites, remove `strip_ansi_codes`, add import |

---

## Task 1: Update Cargo.toml

**Files:**
- Modify: `cel-runtime/Cargo.toml`

- [ ] **Step 1: Replace owo-colors with annotate-snippets**

  In `cel-runtime/Cargo.toml`, remove the `owo-colors` line and add `annotate-snippets`:

  ```toml
  [dependencies]
  anyhow = "1.0"
  typenum = "1.18.0"
  proc-macro2 = { version = "1.0", features = ["span-locations"] }
  quote = "1.0"
  annotate-snippets = "0.12"
  syn = { version = "2.0", features = ["extra-traits", "parsing"] }
  phf = { version = "0.11", features = ["macros"] }
  once_cell = "1.19"
  ```

  (Remove the `owo-colors` line entirely; add `annotate-snippets = "0.12"`.)

- [ ] **Step 2: Fetch the dependency**

  ```
  cargo fetch
  ```

  Expected: fetches annotate-snippets 0.12.x and its transitive deps. No errors.

---

## Task 2: Update error.rs

**Files:**
- Modify: `cel-runtime/src/parser/error.rs`

- [ ] **Step 1: Replace the import block at the top of the file**

  Replace:
  ```rust
  use owo_colors::OwoColorize;
  use proc_macro2::LineColumn;
  ```

  With:
  ```rust
  use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};
  use proc_macro2::LineColumn;
  ```

- [ ] **Step 2: Add the private helper function**

  Insert this function immediately before the `impl CELError` block (after the `CELError` struct definition, around line 72):

  ```rust
  /// Converts a 1-based `line` and 0-based character-count `col` to a byte offset in `source`.
  ///
  /// Returns `source.len()` if the position is past the end of the source.
  ///
  /// - Complexity: O(n) in the length of `source`.
  fn line_col_to_byte_offset(source: &str, line: usize, col: usize) -> usize {
      let mut current_line = 1;
      let mut pos = 0;
      for c in source.chars() {
          if current_line == line {
              break;
          }
          if c == '\n' {
              current_line += 1;
          }
          pos += c.len_utf8();
      }
      for c in source[pos..].chars().take(col) {
          pos += c.len_utf8();
      }
      pos
  }
  ```

- [ ] **Step 3: Replace `format_rustc_style`**

  Replace the entire `format_rustc_style` function (currently lines 98–169) with:

  ```rust
  /// Formats this error in rustc diagnostic style with source context.
  ///
  /// Produces a multi-line string matching Rust compiler diagnostic output,
  /// including the source file location, error message, and a caret indicating
  /// the error position. Uses [annotate-snippets](https://docs.rs/annotate-snippets)
  /// for rendering.
  ///
  /// Pass [`Renderer::plain`] for tests and non-ANSI contexts; pass
  /// [`Renderer::styled`] for terminal output.
  ///
  /// # Examples
  ///
  /// ```no_run
  /// use annotate_snippets::Renderer;
  /// use cel_runtime::parser::CELParser;
  /// use cel_runtime::parser::op_table::OpLookup;
  ///
  /// let source = "10 + 20 30";
  /// let mut parser = CELParser::new(OpLookup::new());
  /// if let Err(e) = parser.parse_str(source) {
  ///     println!("{}", e.format_rustc_style(source, "example.cel", 1, &Renderer::styled()));
  /// }
  /// ```
  pub fn format_rustc_style(
      &self,
      source_code: &str,
      filename: &str,
      start_line: u32,
      renderer: &Renderer,
  ) -> String {
      let start_byte =
          line_col_to_byte_offset(source_code, self.span.start.line, self.span.start.column);
      let end_byte =
          line_col_to_byte_offset(source_code, self.span.end.line, self.span.end.column)
              .max(start_byte + 1);
      let report = [Group::with_title(Level::ERROR.primary_title(self.message.as_str())).element(
          Snippet::source(source_code)
              .path(filename)
              .line_start(start_line as usize)
              .annotation(AnnotationKind::Primary.span(start_byte..end_byte)),
      )];
      renderer.render(&report)
  }
  ```

- [ ] **Step 4: Verify it compiles in isolation**

  ```
  cargo build -p cel-runtime
  ```

  Expected: build fails with "error[E0061]: this function takes 5 arguments but 4 arguments were supplied" for each call site in `mod.rs`. That is correct — the call sites haven't been updated yet. There should be **no** errors about unknown types or missing imports.

  If there are import errors (e.g. `Group` not found), check that you added `annotate-snippets = "0.12"` to `cel-runtime/Cargo.toml` in Task 1 and that the import line is `use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};`.

---

## Task 3: Update call sites in mod.rs

**Files:**
- Modify: `cel-runtime/src/parser/mod.rs`

There are five call sites and one helper to remove.

- [ ] **Step 1: Add Renderer import to the test module**

  The `#[cfg(test)]` module at line 787 currently starts with:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use anyhow;
  ```

  Add the Renderer import:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use annotate_snippets::Renderer;
      use anyhow;
  ```

- [ ] **Step 2: Remove the `strip_ansi_codes` helper**

  Delete the entire function (lines 991–1014):

  ```rust
  /// Helper function to strip ANSI escape codes from a string for testing purposes
  fn strip_ansi_codes(input: &str) -> String {
      let mut result = String::new();
      let mut chars = input.chars().peekable();

      while let Some(ch) = chars.next() {
          if ch == '\x1B' {
              if chars.peek() == Some(&'[') {
                  chars.next();
                  while let Some(ch) = chars.next() {
                      if ch.is_ascii_alphabetic() {
                          break;
                      }
                  }
              } else {
                  result.push(ch);
              }
          } else {
              result.push(ch);
          }
      }

      result
  }
  ```

- [ ] **Step 3: Update `error_formatting` test**

  Replace the body of the `error_formatting` test (the part that calls `format_rustc_style`) so it reads:

  ```rust
  #[test]
  fn error_formatting() {
      let source = "10 + 20 30";
      let mut parser = CELParser::new(OpLookup::new());
      let result = parser.parse_str(source);

      assert!(result.is_err());

      let err = match &result {
          Ok(_) => panic!("expected parse error"),
          Err(e) => e,
      };
      assert_eq!(err.message(), "unexpected token");

      let formatted = err.format_rustc_style(source, "test.cel", 1u32, &Renderer::plain());
      assert!(formatted.contains("error: unexpected token"));
      assert!(formatted.contains("test.cel:1:"));
      assert!(formatted.contains("1 | 10 + 20 30"));
      assert!(formatted.contains("^"));
  }
  ```

- [ ] **Step 4: Update `error_formatting_with_line_offset` test**

  Replace the body:

  ```rust
  #[test]
  fn error_formatting_with_line_offset() {
      let source = "10 + 20 30";
      let mut parser = CELParser::new(OpLookup::new());
      let result = parser.parse_str(source);

      assert!(result.is_err());

      let err = match &result {
          Ok(_) => panic!("expected parse error"),
          Err(e) => e,
      };
      let formatted = err.format_rustc_style(source, "large_file.rs", 42u32, &Renderer::plain());
      assert!(formatted.contains("error: unexpected token"));
      assert!(formatted.contains("large_file.rs:42:"));
      assert!(formatted.contains("42 | 10 + 20 30"));
      assert!(formatted.contains("^"));
  }
  ```

- [ ] **Step 5: Update `print_error_formatting` test**

  Replace the `format_rustc_style` call and the `formatted` variable (remove the `strip_ansi_codes` call — `Renderer::plain()` produces no ANSI codes):

  ```rust
  let formatted_error = err.format_rustc_style(source, file!(), line, &Renderer::plain());
  println!("{}", formatted_error);

  let formatted = formatted_error;
  ```

  The assertions that follow (`formatted.contains(...)`) are unchanged.

- [ ] **Step 6: Update `test_undefined_identifier_error_formatting` test**

  Replace the `format_rustc_style` call:

  ```rust
  let formatted_error = e.format_rustc_style(input, "test.cel", 1, &Renderer::plain());
  assert!(formatted_error.contains("Undefined identifier"));
  assert!(formatted_error.contains("undefined_var"));
  assert!(formatted_error.contains("test.cel"));
  ```

- [ ] **Step 7: Update the playground call site**

  Add a Renderer import to the playground module (line 1290):

  ```rust
  #[cfg(all(test, feature = "playground"))]
  mod playground {
      use super::*;
      use annotate_snippets::Renderer;
  ```

  Then update the call at the `Err` arm in `custom_scope_identifier`:

  ```rust
  Err(e) => println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::styled())),
  ```

---

## Task 4: Verify

- [ ] **Step 1: Full build**

  ```
  cargo build --workspace
  ```

  Expected: compiles cleanly, zero errors, zero warnings.

- [ ] **Step 2: Full test suite**

  ```
  cargo test --workspace
  ```

  Expected: all tests pass. The `error_formatting`, `error_formatting_with_line_offset`, `print_error_formatting`, and `test_undefined_identifier_error_formatting` tests should all pass with plain-rendered output.

- [ ] **Step 3: Lint**

  ```
  cargo clippy --workspace -- -D warnings
  ```

  Expected: zero warnings, zero errors.

- [ ] **Step 4: Doc tests**

  ```
  cargo test --doc --workspace
  ```

  Expected: all doc tests pass (including the new example in `format_rustc_style`).

---

## Task 5: Commit

- [ ] **Step 1: Stage and commit**

  ```
  git add cel-runtime/Cargo.toml
  git add cel-runtime/src/parser/error.rs
  git add cel-runtime/src/parser/mod.rs
  git commit -m "Migrate format_rustc_style to annotate-snippets, remove owo-colors"
  ```

---

## Notes for implementers

- The `annotate-snippets` API is in flux between minor versions. If `Group::with_title(...)` does not exist, check the crate docs for the correct constructor — alternatives include `Level::ERROR.primary_title(text).element(snippet)` if `Title` has an `.element()` method in the resolved version.
- `AnnotationKind::Primary.span(range)` creates an `Annotation`. The span range is byte offsets within `source_code`, not character counts.
- `Renderer::plain()` produces no ANSI escape codes; `Renderer::styled()` detects terminal capability and adds color.
- `line_col_to_byte_offset` uses 1-based lines and 0-based character-count columns, matching `proc_macro2::LineColumn`.
