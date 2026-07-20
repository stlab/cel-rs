//! Computes LSP diagnostics for pm-lang source text. No LSP transport knowledge lives here —
//! see `crate::dispatch` for that — so this module's tests exercise the diagnostic logic
//! directly, per the design doc's testing strategy (handler-level unit tests over full protocol
//! round-trips where possible).

use cel_parser::{CELError, SourceSpan};
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use pm_lang::{PmAstParser, TypeRegistry, check_sheet};

/// Parses and type-checks pm-lang `source`, returning every recovered syntax and type error as
/// an LSP diagnostic.
///
/// - Postcondition: returns one [`Diagnostic`] per [`cel_parser::ParseError`] recovered by
///   [`PmAstParser::parse_str`] (syntax errors, including the single error produced when the
///   whole parse aborts instead of recovering one item) or returned by [`check_sheet`] (type
///   errors); returns an empty `Vec` for a syntactically and semantically clean sheet.
///
/// # Examples
///
/// ```
/// use pm_lsp::diagnostics::diagnostics_for_source;
///
/// assert!(diagnostics_for_source("sheet s { cell x: i32 = 1; }").is_empty());
/// assert_eq!(diagnostics_for_source("sheet s { cell x: i32 = 1.0; }").len(), 1);
/// ```
pub fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
    let mut parser = PmAstParser::new();
    let sheet = match parser.parse_str(source) {
        Ok(sheet) => sheet,
        Err(e) => return vec![to_diagnostic(e.into())],
    };
    let type_errors = check_sheet(&sheet, &TypeRegistry::new());
    sheet
        .errors
        .into_iter()
        .chain(type_errors)
        .map(|e| to_diagnostic(e.into()))
        .collect()
}

/// Converts a [`CELError`] into an LSP [`Diagnostic`] at error severity.
fn to_diagnostic(error: CELError) -> Diagnostic {
    Diagnostic {
        range: to_range(error.span()),
        severity: Some(DiagnosticSeverity::ERROR),
        message: error.message().to_string(),
        ..Default::default()
    }
}

/// Converts a [`SourceSpan`] (1-based lines, character-offset columns) into an LSP [`Range`]
/// (0-based lines, UTF-16-code-unit columns).
///
/// - Precondition: no character on `span`'s line(s) before `span` lies outside the Basic
///   Multilingual Plane — pm-lang source is overwhelmingly ASCII, so a character-offset ≈
///   UTF-16-code-unit-offset approximation is accepted here rather than computed exactly.
fn to_range(span: SourceSpan) -> Range {
    Range {
        start: Position {
            line: (span.start.line - 1) as u32,
            character: span.start.column as u32,
        },
        end: Position {
            line: (span.end.line - 1) as u32,
            character: span.end.column as u32,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_sheet_has_no_diagnostics() {
        assert!(diagnostics_for_source("sheet s { cell x: i32 = 1; }").is_empty());
    }

    #[test]
    fn type_mismatched_cell_initializer_is_a_diagnostic() {
        // Unsuffixed float literal defaults to f64, not the declared i32 — same fixture used by
        // pm-lang's own `check_sheet` test suite (pm-lang/src/typecheck.rs).
        let diags = diagnostics_for_source("sheet s { cell x: i32 = 1.0; }");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn recovered_syntax_error_is_a_diagnostic() {
        // Malformed cell declaration; PmAstParser recovers it as one `Sheet.errors` entry
        // (pm-lang/src/ast_parser.rs's `parse_malformed_cell_is_recorded_as_an_error_item`).
        assert_eq!(
            diagnostics_for_source("sheet s { cell x unknown_syntax }").len(),
            1
        );
    }

    #[test]
    fn unrecoverable_parse_failure_is_a_single_diagnostic() {
        // Missing the `sheet` keyword entirely is a structural error outside any item —
        // PmAstParser::parse_str's documented `Err` case, not a `Sheet.errors` recovery.
        assert_eq!(diagnostics_for_source("not a sheet at all").len(), 1);
    }
}
