//! Parse error type with message and source span for CEL.
//!
//! Uses a [`SourceSpan`] (line/column only) so errors are `Send + Sync` and can
//! be used from async execution. Use [`SourceSpan::from_proc_macro2`] to extract
//! location from a `proc_macro2::Span` when building errors in the parser.

use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};
use proc_macro2::LineColumn;

/// Source region as start/end line and column.
///
/// Uses [`proc_macro2::LineColumn`] for positions (1-based line, 0-based column).
/// This type is `Send + Sync`. Build it from a `proc_macro2::Span` via
/// [`SourceSpan::from_proc_macro2`] when you have one (e.g. in the parser).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    /// Start position (inclusive).
    pub start: LineColumn,
    /// End position (inclusive).
    pub end: LineColumn,
}

impl Default for SourceSpan {
    fn default() -> Self {
        SourceSpan {
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn { line: 1, column: 0 },
        }
    }
}

impl SourceSpan {
    /// Builds a span from raw line/column values.
    ///
    /// Lines are 1-based, columns are 0-based (matching [`proc_macro2::LineColumn`]).
    pub fn new(start_line: usize, start_column: usize, end_line: usize, end_column: usize) -> Self {
        SourceSpan {
            start: LineColumn {
                line: start_line,
                column: start_column,
            },
            end: LineColumn {
                line: end_line,
                column: end_column,
            },
        }
    }

    /// Extracts start/end line and column from a `proc_macro2::Span`.
    ///
    /// Use this when creating errors in the parser or other code that has a
    /// `proc_macro2::Span`; the result is `Send + Sync` and can be stored in
    /// [`CELError`] for use from async or other threads.
    pub fn from_proc_macro2(span: proc_macro2::Span) -> Self {
        SourceSpan {
            start: span.start(),
            end: span.end(),
        }
    }
}

/// A CEL parse error with a message and source location.
///
/// Uses a [`SourceSpan`] (line/column only) so the error is `Send + Sync` and
/// can be used from async execution or reported across thread boundaries.
#[derive(Clone, Debug)]
pub struct CELError {
    message: String,
    span: SourceSpan,
}

/// Converts a [`SourceSpan`] (1-based lines, 0-based character columns) to a byte-offset
/// range within `source`.
///
/// Returns `source.len()..source.len()` if the span lies past the end of `source`.
///
/// - Complexity: O(n) in the length of `source`.
fn span_to_byte_range(source: &str, span: SourceSpan) -> std::ops::Range<usize> {
    let start_line_byte: usize = source
        .split_inclusive('\n')
        .take(span.start.line - 1)
        .map(str::len)
        .sum();
    let start_byte = start_line_byte
        + source[start_line_byte..]
            .chars()
            .take(span.start.column)
            .map(char::len_utf8)
            .sum::<usize>();
    let end_byte = if span.end.line == span.start.line {
        start_byte
            + source[start_byte..]
                .chars()
                .take(span.end.column - span.start.column)
                .map(char::len_utf8)
                .sum::<usize>()
    } else {
        let end_line_byte = start_byte
            + source[start_byte..]
                .split_inclusive('\n')
                .take(span.end.line - span.start.line)
                .map(str::len)
                .sum::<usize>();
        end_line_byte
            + source[end_line_byte..]
                .chars()
                .take(span.end.column)
                .map(char::len_utf8)
                .sum::<usize>()
    };
    start_byte..end_byte
}

impl CELError {
    /// Creates a new error with the given message and source span.
    pub fn new(message: impl Into<String>, span: SourceSpan) -> Self {
        CELError {
            message: message.into(),
            span,
        }
    }

    /// Creates a new error from a message and a `proc_macro2::Span`.
    ///
    /// Extracts line/column from the span so the resulting error is `Send + Sync`.
    pub fn with_proc_macro_span(message: impl Into<String>, span: proc_macro2::Span) -> Self {
        CELError::new(message, SourceSpan::from_proc_macro2(span))
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the source span for this error.
    pub fn span(&self) -> SourceSpan {
        self.span
    }

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
    /// ```rust
    /// use annotate_snippets::Renderer;
    /// use cel_runtime::parser::CELParser;
    /// use cel_runtime::parser::op_table::OpLookup;
    ///
    /// let line = line!() + 1;
    /// let source = r#"
    ///     10 + 20 30
    /// "#;
    /// let mut parser = CELParser::new(OpLookup::new());
    /// if let Err(e) = parser.parse_str(source) {
    ///     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::styled()));
    /// }
    /// ```
    pub fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        let byte_range = span_to_byte_range(source_code, self.span);
        let report = [
            Group::with_title(Level::ERROR.primary_title(self.message.as_str())).element(
                Snippet::source(source_code)
                    .path(filename)
                    .line_start(start_line as usize)
                    .annotation(AnnotationKind::Primary.span(byte_range)),
            ),
        ];
        renderer.render(&report)
    }
}

impl std::fmt::Display for CELError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CELError {}
