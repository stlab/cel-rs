# Design: Replace manual rustc-style formatting with annotate-snippets

**Date:** 2026-06-03
**File:** `cel-runtime/src/parser/error.rs`

## Goal

Replace the hand-rolled rustc-style diagnostic formatter in `CELError::format_rustc_style` with the [`annotate-snippets`](https://docs.rs/annotate-snippets/latest/annotate_snippets/) crate, which is the library Rust itself now uses for compiler diagnostics. Remove the `owo-colors` dependency that the manual formatter required.

## Scope

- `cel-runtime/Cargo.toml` — dependency changes only
- `cel-runtime/src/parser/error.rs` — implementation changes
- `cel-runtime/src/parser/mod.rs` — test updates only

No other files are affected.

## Dependencies

- **Add:** `annotate-snippets = "0.12"` to `cel-runtime/Cargo.toml`
- **Remove:** `owo-colors` from `cel-runtime/Cargo.toml` (only used in `error.rs`, fully replaced)

## API change

The signature of `format_rustc_style` gains a `renderer` parameter:

```rust
// Before
pub fn format_rustc_style(&self, source_code: &str, filename: &str, start_line: u32) -> String

// After
pub fn format_rustc_style(&self, source_code: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String
```

Callers use `Renderer::styled()` for terminal output and `Renderer::plain()` for tests and non-ANSI contexts. The return type remains `String` (annotate-snippets `render()` returns `String`).

## Implementation

### Private helper: `line_col_to_byte_offset`

`SourceSpan` stores `proc_macro2::LineColumn` positions — 1-based lines, 0-based character-count columns. annotate-snippets 0.12 requires byte-offset spans (`Range<usize>`) within the source string.

```rust
/// Converts a 1-based line and 0-based character column to a byte offset in `source`.
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

This replaces the current `lines: Vec<&str>` allocation with a single char-walk.

### `format_rustc_style` body

```rust
pub fn format_rustc_style(&self, source_code: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String {
    let start_byte = line_col_to_byte_offset(source_code, self.span.start.line, self.span.start.column);
    let end_byte = line_col_to_byte_offset(source_code, self.span.end.line, self.span.end.column);
    let message = Level::ERROR.title(&self.message)
        .snippet(
            Snippet::source(source_code)
                .line_start(start_line as usize)
                .origin(filename)
                .annotation(Level::ERROR.span(start_byte..end_byte))
        );
    renderer.render(message)
}
```

The `use owo_colors::OwoColorize` import is removed. Imports for `annotate_snippets::{Level, Renderer, Snippet}` (or similar — exact paths to be confirmed against the 0.12 API) are added.

## Test changes (`cel-runtime/src/parser/mod.rs`)

The `strip_ansi_codes` helper and all its call sites are removed. Each test that called `format_rustc_style` is updated to pass `&Renderer::plain()`. The substring assertions (`"error: ..."`, `"filename:line:"`, `"N | source line"`, `"^"`) are unchanged — annotate-snippets produces all of them in plain mode.

## Doc comment update

The `format_rustc_style` doc comment is updated to:
- Document the new `renderer` parameter (use `Renderer::plain()` for tests/non-ANSI, `Renderer::styled()` for terminals).
- Remove the manual `# References` link (the library itself is the reference).
- Update the contract to match the new implementation.

## What does NOT change

- `SourceSpan` struct and all its methods
- `CELError` struct and all other methods
- The observable output format (annotate-snippets matches rustc diagnostic style)
- The `String` return type
