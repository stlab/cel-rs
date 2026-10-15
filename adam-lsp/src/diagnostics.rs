//! Computes LSP diagnostics for adam-lang source text. No LSP transport knowledge lives here —
//! see `crate::dispatch` for that — so this module's tests exercise the diagnostic logic
//! directly, per the design doc's testing strategy (handler-level unit tests over full protocol
//! round-trips where possible).

use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
use cel_parser::{CELError, SourceSpan};
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Parses and type-checks adam-lang `source`, returning every recovered syntax and type error as
/// an LSP diagnostic.
///
/// - Postcondition: returns one [`Diagnostic`] per [`cel_parser::ParseError`] recovered by
///   [`AdamAstParser::parse_str`] (syntax errors, including the single error produced when the
///   whole parse aborts instead of recovering one item) or returned by [`check_sheet`] (type
///   errors); returns an empty `Vec` for a syntactically and semantically clean sheet.
///
/// The environment explicitly includes the standard library and optional Euclid geometry,
/// matching the executing web/book hosts. Unresolved call-result shapes remain best-effort.
///
/// - Complexity: O(n + s²) in source length and installed library signatures, with no caching.
///
/// # Examples
///
/// ```
/// use adam_lsp::diagnostics::diagnostics_for_source;
///
/// assert!(diagnostics_for_source("sheet s { cell x: i32 = 1; }").is_empty());
/// assert_eq!(diagnostics_for_source("sheet s { cell x: i32 = 1.0; }").len(), 1);
/// ```
pub fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
    let mut parser = AdamAstParser::new();
    let sheet = match parser.parse_str(source) {
        Ok(sheet) => sheet,
        Err(e) => return vec![to_diagnostic(e.into())],
    };
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let type_errors = check_sheet(&sheet, &types, &lookup);
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
///   Multilingual Plane — adam-lang source is overwhelmingly ASCII, so a character-offset ≈
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

    /// The LSP uses the same standard and Euclid metadata as executing hosts.
    #[test]
    fn lsp_geometry_fields_and_members_have_matching_diagnostics() {
        let valid = "sheet s { out x := (|p: Point2D| p.x)(Point2D { y: 2.0, x: 1.0 }); out n := min(3.0, 4.0); }";
        assert!(
            diagnostics_for_source(valid).is_empty(),
            "{:?}",
            diagnostics_for_source(valid)
        );
        let invalid = "sheet s { out p := Point2D { x: true, y: 1.0 }; }";
        let diagnostics = diagnostics_for_source(invalid);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let invalid = "sheet s { out p := Point2D { x: 1.0, y: 1.0 }.translate(1.0); }";
        assert!(!diagnostics_for_source(invalid).is_empty());
        for actual in [
            include_str!("../../cel-euclid/examples/crop_straighten.adm2"),
            include_str!("../../cel-euclid/examples/pan_zoom.adm2"),
        ] {
            let diagnostics = diagnostics_for_source(actual);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        }
    }

    /// Reports missing host schemas and still checks field expressions through the LSP path.
    #[test]
    fn unknown_record_and_child_errors_reach_lsp_diagnostics() {
        let diagnostics =
            diagnostics_for_source("sheet s {\n    out value := Missing { x: true + 1 }.x;\n}");
        assert_eq!(diagnostics.len(), 2);
        let missing = diagnostics
            .iter()
            .find(|d| d.message.contains("Missing"))
            .unwrap();
        assert_eq!(missing.range.start, Position::new(1, 17));
        assert_eq!(missing.range.end, Position::new(1, 24));
        assert!(diagnostics.iter().any(|d| d.message.contains("operand")));
    }

    #[test]
    fn clean_sheet_has_no_diagnostics() {
        assert!(diagnostics_for_source("sheet s { cell x: i32 = 1; }").is_empty());
    }

    #[test]
    fn type_mismatched_cell_initializer_is_a_diagnostic() {
        // Unsuffixed float literal defaults to f64, not the declared i32 — same fixture used by
        // adam-lang's own `check_sheet` test suite (adam-lang/src/typecheck.rs).
        let diags = diagnostics_for_source("sheet s { cell x: i32 = 1.0; }");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn recovered_syntax_error_is_a_diagnostic() {
        // Malformed cell declaration; AdamAstParser recovers it as one `Sheet.errors` entry
        // (adam-lang/src/ast_parser.rs's `parse_malformed_cell_is_recorded_as_an_error_item`).
        assert_eq!(
            diagnostics_for_source("sheet s { cell x unknown_syntax }").len(),
            1
        );
    }

    #[test]
    fn unrecoverable_parse_failure_is_a_single_diagnostic() {
        // Missing the `sheet` keyword entirely is a structural error outside any item —
        // AdamAstParser::parse_str's documented `Err` case, not a `Sheet.errors` recovery.
        assert_eq!(diagnostics_for_source("not a sheet at all").len(), 1);
    }

    #[test]
    fn filter_clause_with_matching_types_has_no_diagnostics() {
        assert!(diagnostics_for_source("sheet s { cell a: i32 = 1 filter _; }").is_empty());
    }

    #[test]
    fn filter_clause_without_underscore_is_a_diagnostic() {
        let diags = diagnostics_for_source("sheet s { cell a: i32 = 1 filter 1; }");
        assert_eq!(diags.len(), 1);
    }
}
