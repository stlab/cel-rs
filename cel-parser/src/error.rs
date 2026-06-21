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
    /// End position (exclusive; one past the last character, matching `proc_macro2::Span::end()`).
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::SourceSpan;
    ///
    /// let span = SourceSpan::new(1, 0, 1, 5);
    /// assert_eq!(span.start.line, 1);
    /// assert_eq!(span.end.column, 5);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use proc_macro2::Span;
    /// use cel_parser::SourceSpan;
    ///
    /// let span = SourceSpan::from_proc_macro2(Span::call_site());
    /// assert_eq!(span.start.line, span.end.line);
    /// ```
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
/// - Complexity: O(n) in the length of `source`.
fn span_to_byte_range(source: &str, span: SourceSpan) -> std::ops::Range<usize> {
    debug_assert!(
        span.start.line >= 1,
        "`span.start.line` must be 1-based (≥ 1)"
    );
    debug_assert!(
        span.end.line > span.start.line
            || (span.end.line == span.start.line && span.end.column >= span.start.column),
        "`span.end` must not precede `span.start`"
    );
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
    debug_assert!(
        end_byte <= source.len(),
        "computed end_byte exceeds source.len()"
    );
    start_byte..end_byte
}

impl CELError {
    /// Creates a new error with the given message and source span.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::CELError;
    /// use cel_parser::SourceSpan;
    ///
    /// let span = SourceSpan::default();
    /// let e = CELError::new("unexpected token", span);
    /// assert_eq!(e.message(), "unexpected token");
    /// ```
    pub fn new(message: impl Into<String>, span: SourceSpan) -> Self {
        CELError {
            message: message.into(),
            span,
        }
    }

    /// Returns the error message.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::CELError;
    /// use cel_parser::SourceSpan;
    ///
    /// let e = CELError::new("bad input", SourceSpan::default());
    /// assert_eq!(e.message(), "bad input");
    /// ```
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the source span for this error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::CELError;
    /// use cel_parser::SourceSpan;
    ///
    /// let span = SourceSpan::default();
    /// let e = CELError::new("bad input", span);
    /// assert_eq!(e.span(), span);
    /// ```
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
    /// use cel_parser::CELParser;
    /// use cel_parser::op_table::OpLookup;
    /// use cel_parser::CELError;
    ///
    /// let line = line!() + 1;
    /// let source = r#"
    ///     10 + 20 30
    /// "#;
    /// let mut parser = CELParser::new(OpLookup::new());
    /// if let Err(e) = parser.parse_str(source) {
    ///     let e: CELError = e.into();
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

/// A parse error carrying the original `proc_macro2::Span` of the offending token or
/// expression start, plus an optional end span for range errors.
///
/// Used as the return type of all parser methods. Not `Send + Sync` because
/// `proc_macro2::Span` wraps a compiler-internal handle that is only valid on
/// the proc-macro thread. Convert to [`CELError`] via `From` when the error
/// must cross thread boundaries or be stored for async reporting.
#[derive(Clone, Debug)]
pub struct ParseError {
    message: String,
    span: proc_macro2::Span,
    end_span: Option<proc_macro2::Span>,
}

impl ParseError {
    /// Creates a new parse error with the given message and token span.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use proc_macro2::Span;
    /// use cel_parser::ParseError;
    ///
    /// let e = ParseError::new("unexpected token", Span::call_site());
    /// assert_eq!(e.message(), "unexpected token");
    /// ```
    pub fn new(message: impl Into<String>, span: proc_macro2::Span) -> Self {
        ParseError {
            message: message.into(),
            span,
            end_span: None,
        }
    }

    /// Creates a parse error spanning a sub-expression from `start` to `end`.
    ///
    /// Use this for op-lookup failures where the error implicates a full
    /// sub-expression rather than a single token. At runtime the two spans
    /// are merged into a contiguous underline; at compile time two separate
    /// `compile_error!()` diagnostics are emitted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use proc_macro2::Span;
    /// use cel_parser::ParseError;
    ///
    /// let start = Span::call_site();
    /// let end = Span::call_site();
    /// let e = ParseError::new_range("type mismatch", start, end);
    /// assert_eq!(e.message(), "type mismatch");
    /// assert!(e.end_span().is_some());
    /// ```
    pub fn new_range(
        message: impl Into<String>,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
    ) -> Self {
        ParseError {
            message: message.into(),
            span: start,
            end_span: Some(end),
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the `proc_macro2::Span` of the offending token.
    ///
    /// Use this span with `quote_spanned!` in proc-macro code to attach the
    /// `compile_error!` to the exact source location.
    pub fn span(&self) -> proc_macro2::Span {
        self.span
    }

    /// Returns the end span of this error, or `None` for single-point errors.
    ///
    /// `Some` for errors created with [`new_range`](Self::new_range);
    /// `None` for errors created with [`new`](Self::new).
    pub fn end_span(&self) -> Option<proc_macro2::Span> {
        self.end_span
    }

    /// Formats this error in rustc diagnostic style with source context.
    ///
    /// Identical contract to [`CELError::format_rustc_style`]; prefer calling
    /// this directly on a `ParseError` rather than converting to `CELError`
    /// first when you have the source text at hand.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use annotate_snippets::Renderer;
    /// use cel_parser::CELParser;
    /// use cel_parser::OpLookup;
    ///
    /// let line = line!() + 1;
    /// let source = "10 + 20 30";
    /// let mut parser = CELParser::new(OpLookup::new());
    /// if let Err(e) = parser.parse_str(source) {
    ///     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
    /// }
    /// ```
    pub fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        let source_span = SourceSpan {
            start: self.span.start(),
            end: self.end_span.unwrap_or(self.span).end(),
        };
        let byte_range = span_to_byte_range(source_code, source_span);
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

/// Converts a [`ParseError`] to a [`CELError`] by extracting the
/// [`SourceSpan`] from the token span.
///
/// Use this at async/runtime boundaries where `Send + Sync` is required.
impl From<ParseError> for CELError {
    fn from(e: ParseError) -> Self {
        let source_span = SourceSpan {
            start: e.span.start(),
            end: e.end_span.unwrap_or(e.span).end(),
        };
        CELError::new(e.message, source_span)
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// A runtime error context carrying the source span of the failing operation.
///
/// Add this as anyhow context with `.context(SpanContext::new(span))` when wrapping
/// an op closure. Retrieve it from an `anyhow::Error` with `e.downcast_ref::<SpanContext>()`.
pub struct SpanContext {
    span: SourceSpan,
}

impl SpanContext {
    /// Creates a new span context for the given source region.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::{SourceSpan, SpanContext};
    ///
    /// let span = SourceSpan::new(1, 0, 1, 5);
    /// let ctx = SpanContext::new(span);
    /// assert_eq!(ctx.span(), span);
    /// ```
    pub fn new(span: SourceSpan) -> Self {
        SpanContext { span }
    }

    /// Returns the source span.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::{SourceSpan, SpanContext};
    ///
    /// let span = SourceSpan::new(2, 3, 2, 7);
    /// let ctx = SpanContext::new(span);
    /// assert_eq!(ctx.span(), span);
    /// ```
    pub fn span(&self) -> SourceSpan {
        self.span
    }

    /// Formats a runtime error message with rustc-style source annotation.
    ///
    /// Delegates to `CELError::format_rustc_style` using `self.span` and `message`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use annotate_snippets::Renderer;
    /// use cel_parser::{SourceSpan, SpanContext};
    ///
    /// let source = "1i32 + 2i32";
    /// let span = SourceSpan::new(1, 5, 1, 6);
    /// let ctx = SpanContext::new(span);
    /// let output = ctx.format_rustc_style("arithmetic overflow", source, "test.cel", 1, &Renderer::plain());
    /// assert!(output.contains("arithmetic overflow"));
    /// ```
    pub fn format_rustc_style(
        &self,
        message: &str,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        CELError::new(message, self.span).format_rustc_style(
            source_code,
            filename,
            start_line,
            renderer,
        )
    }
}

impl std::fmt::Display for SpanContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "at {}:{}-{}:{}",
            self.span.start.line, self.span.start.column, self.span.end.line, self.span.end.column
        )
    }
}

impl std::fmt::Debug for SpanContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanContext")
            .field("span", &self.span)
            .finish()
    }
}

impl std::error::Error for SpanContext {}

/// Extension trait that adds rustc-style formatting to `anyhow::Error`.
///
/// Import this trait to call `.format_rustc_style(...)` directly on an `anyhow::Error`.
/// If the error carries a [`SpanContext`] (added during op execution with the
/// `span-diagnostics` feature), the output includes a source-location annotation.
/// Otherwise it falls back to `self.to_string()`.
///
/// # Examples
///
/// ```rust
/// use annotate_snippets::Renderer;
/// use cel_parser::FormatRustcStyle;
///
/// let err = anyhow::anyhow!("something went wrong");
/// let output = err.format_rustc_style("1 + 2", "example.cel", 1, &Renderer::plain());
/// assert_eq!(output, "something went wrong");
/// ```
pub trait FormatRustcStyle {
    /// Formats in rustc diagnostic style.
    ///
    /// If the error carries a [`SpanContext`], produces a multi-line caret diagnostic.
    /// Otherwise returns `self.to_string()`.
    fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String;
}

impl FormatRustcStyle for anyhow::Error {
    fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        if let Some(ctx) = self.downcast_ref::<SpanContext>() {
            let message = self
                .source()
                .map(|e| e.to_string())
                .unwrap_or_else(|| self.to_string());
            ctx.format_rustc_style(&message, source_code, filename, start_line, renderer)
        } else {
            self.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use annotate_snippets::Renderer;
    use proc_macro2::Span;

    #[test]
    fn parse_error_message() {
        let e = ParseError::new("bad token", Span::call_site());
        assert_eq!(e.message(), "bad token");
    }

    #[test]
    fn parse_error_display() {
        let e = ParseError::new("bad token", Span::call_site());
        assert_eq!(e.to_string(), "bad token");
    }

    #[test]
    fn parse_error_into_cel_error() {
        let e = ParseError::new("bad token", Span::call_site());
        let cel: CELError = e.into();
        assert_eq!(cel.message(), "bad token");
        assert_eq!(cel.span(), SourceSpan::from_proc_macro2(Span::call_site()));
    }

    #[test]
    fn parse_error_format_rustc_style() {
        let source = "10 + 20 30";
        let e = ParseError::new("unexpected token", Span::call_site());
        let formatted = e.format_rustc_style(source, "test.cel", 1, &Renderer::plain());
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("test.cel"));
    }

    #[test]
    fn parse_error_new_range_has_end_span() {
        let start = Span::call_site();
        let end = Span::call_site();
        let e = ParseError::new_range("type mismatch", start, end);
        assert_eq!(e.message(), "type mismatch");
        assert!(e.end_span().is_some());
    }

    #[test]
    fn parse_error_new_range_cel_error_merges_spans() {
        let ts: proc_macro2::TokenStream = r#""Hello" 32.0"#.parse().unwrap();
        let mut iter = ts.into_iter();
        let start_tok = iter.next().unwrap();
        let end_tok = iter.next().unwrap();
        let start = start_tok.span();
        let end = end_tok.span();

        let e = ParseError::new_range("type mismatch", start, end);
        let cel: CELError = e.into();

        assert_eq!(cel.message(), "type mismatch");
        assert_eq!(
            cel.span().start,
            start.start(),
            "CELError span should start at expression start"
        );
        assert_eq!(
            cel.span().end,
            end.end(),
            "CELError span should end at expression end"
        );
    }

    #[test]
    fn parse_error_new_has_no_end_span() {
        let e = ParseError::new("bad token", Span::call_site());
        assert!(e.end_span().is_none());
    }

    #[test]
    fn span_to_byte_range_basic() {
        let source = "hello world";
        assert_eq!(
            span_to_byte_range(source, SourceSpan::new(1, 6, 1, 11)),
            6..11
        );
    }

    #[test]
    fn span_to_byte_range_empty_span() {
        let source = "hello";
        assert_eq!(
            span_to_byte_range(source, SourceSpan::new(1, 2, 1, 2)),
            2..2
        );
    }

    #[test]
    fn span_to_byte_range_start_past_end_of_source() {
        let source = "hi";
        let r = span_to_byte_range(source, SourceSpan::new(5, 0, 5, 3));
        assert_eq!(r, source.len()..source.len());
    }

    #[test]
    fn span_to_byte_range_end_past_end_of_source() {
        let source = "hi";
        let r = span_to_byte_range(source, SourceSpan::new(1, 0, 1, 100));
        assert!(r.start <= r.end && r.end <= source.len());
    }

    #[test]
    fn span_context_display_shows_span_location() {
        let span = SourceSpan::new(1, 0, 1, 5);
        let ctx = SpanContext::new(span);
        let s = ctx.to_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn span_context_span_roundtrip() {
        let span = SourceSpan::new(2, 3, 2, 7);
        let ctx = SpanContext::new(span);
        assert_eq!(ctx.span(), span);
    }

    #[test]
    fn span_context_format_rustc_style_contains_message_and_location() {
        let source = "1i32 + 2i32";
        let span = SourceSpan::new(1, 5, 1, 6);
        let ctx = SpanContext::new(span);
        let output = ctx.format_rustc_style(
            "arithmetic overflow",
            source,
            "test.cel",
            1,
            &Renderer::plain(),
        );
        assert!(
            output.contains("arithmetic overflow"),
            "expected message in output:\n{output}"
        );
        assert!(
            output.contains("test.cel"),
            "expected filename in output:\n{output}"
        );
    }

    #[test]
    fn format_rustc_style_with_span_context_uses_span() {
        let source = "1i32 + 2i32";
        let span = SourceSpan::new(1, 5, 1, 6);
        let inner = anyhow::anyhow!("arithmetic overflow");
        let wrapped = inner.context(SpanContext::new(span));
        let output = FormatRustcStyle::format_rustc_style(
            &wrapped,
            source,
            "test.cel",
            1,
            &Renderer::plain(),
        );
        assert!(
            output.contains("arithmetic overflow"),
            "expected message:\n{output}"
        );
        assert!(output.contains("test.cel"), "expected filename:\n{output}");
    }

    #[test]
    fn format_rustc_style_without_span_context_falls_back_to_to_string() {
        let err = anyhow::anyhow!("something went wrong");
        let output = FormatRustcStyle::format_rustc_style(
            &err,
            "unused source",
            "unused.cel",
            1,
            &Renderer::plain(),
        );
        assert_eq!(output, "something went wrong");
    }
}
