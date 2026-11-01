//! Checks Adam source text without executing it or opening an LSP transport.
//!
//! ```rust
//! let diagnostics = adam_lsp::diagnostics::diagnostics_for_source(
//!     "sheet example { cell x: i32 = true; }",
//! );
//! assert!(!diagnostics.is_empty());
//! ```

use adam_lang::{
    AdamAstParser, ModuleDiagnostic, ModulePath, ModuleSet, TypeRegistry, ast, check_module,
    check_sheet,
};
use cel_parser::{CELError, SourceSpan};
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Parses and checks one complete Adam module without executing its expressions.
///
/// Returns every recovered syntax error. Checks syntactically clean modules,
/// including local component definitions and named bindings, with [`check_module`].
/// A single root without component uses retains [`check_sheet`]'s recovered
/// syntax/type diagnostics and expression-level ranges.
/// External imports require the explicit source collection accepted by
/// [`diagnostics_for_module`]; this convenience checker does no source discovery.
///
/// The environment explicitly includes the standard library and optional Euclid geometry,
/// matching the executing web/book hosts. Unresolved call-result shapes remain best-effort.
/// Diagnostic columns count UTF-16 code units in the supplied text.
///
/// - Complexity: parsing, environment setup and module checking costs plus O(en + b)
///   diagnostic projection time for source bytes n, e errors and b message bytes;
///   space for parsed/checker state and returned diagnostics, with no caching.
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
    let module = match parser.parse_module_str(source) {
        Ok(module) => module,
        Err(e) => return vec![to_diagnostic(e.into(), source)],
    };
    if let [ast::ModuleItem::Sheet(sheet)] = module.items.as_slice()
        && !requires_module_checking(sheet)
    {
        let (types, lookup) = configured_environment();
        return sheet
            .errors
            .iter()
            .cloned()
            .chain(check_sheet(sheet, &types, &lookup))
            .map(|error| to_diagnostic(error.into(), source))
            .collect();
    }
    if !module.errors.is_empty() {
        return module
            .errors
            .into_iter()
            .map(|e| to_diagnostic(e.into(), source))
            .collect();
    }
    drop(module);
    let path: ModulePath = "document".parse().expect("valid internal module path");
    let mut modules = ModuleSet::new();
    if let Err(error) = modules.insert_source(path.clone(), "<document>", source) {
        return vec![to_module_diagnostic(error)];
    }
    diagnostics_for_module(&path, &modules)
        .into_iter()
        .map(to_module_diagnostic)
        .collect()
}

/// Identifies component definitions and uses, including conditional/default branches.
///
/// - Complexity: O(n) time for n sheet and branch items; O(1) additional space.
fn requires_module_checking(sheet: &ast::Sheet) -> bool {
    sheet.parameters.is_some()
        || sheet.items.iter().any(|item| match item {
            ast::SheetItem::RelationshipUse(_) => true,
            ast::SheetItem::Conditional(conditional) => conditional
                .branches
                .iter()
                .map(|branch| &branch.relationships)
                .chain(
                    conditional
                        .default
                        .iter()
                        .map(|branch| &branch.relationships),
                )
                .flatten()
                .any(|item| matches!(item, ast::BranchItem::RelationshipUse(_))),
            _ => false,
        })
}

/// Checks a registered module collection without executing defaults, methods or match keys.
///
/// Uses the same standard and Euclid libraries as [`diagnostics_for_source`].
/// Includes all registered definitions and retains the actual source owners.
/// An unknown requested module returns a diagnostic, never a source-only fallback.
/// - Complexity: the configured libraries' setup plus [`check_module`]'s analysis cost.
///
/// # Examples
///
/// ```
/// let path = "app".parse().unwrap();
/// let mut modules = adam_lang::ModuleSet::new();
/// modules.insert_source(path, "file:///app.adm2", "sheet app {}").unwrap();
/// assert!(adam_lsp::diagnostics::diagnostics_for_module(
///     &"app".parse().unwrap(), &modules
/// ).is_empty());
/// ```
pub fn diagnostics_for_module(module: &ModulePath, modules: &ModuleSet) -> Vec<ModuleDiagnostic> {
    let (types, lookup) = configured_environment();
    check_module(module, modules, &types, &lookup)
}

/// Creates the common source-only and module-aware nonexecuting type environment.
///
/// - Complexity: O(s²) for s installed operation signatures.
fn configured_environment() -> (TypeRegistry, cel_parser::OpLookup) {
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    (types, lookup)
}

/// Converts a [`CELError`] against its original text into an LSP diagnostic.
///
/// - Complexity: O(n + m) time for source-prefix bytes n and message bytes m;
///   O(m) additional space.
fn to_diagnostic(error: CELError, source: &str) -> Diagnostic {
    Diagnostic {
        range: source_range(source, error.span()),
        severity: Some(DiagnosticSeverity::ERROR),
        message: error.message().to_string(),
        ..Default::default()
    }
}

/// Projects a single-source module error without inventing an editor URI.
///
/// - Complexity: O(n + b) time and O(b) space for source-prefix bytes n and
///   message/label bytes b.
fn to_module_diagnostic(error: ModuleDiagnostic) -> Diagnostic {
    let range = error
        .labels
        .iter()
        .find_map(|label| label.location.as_ref())
        .map(|location| source_range(location.source.text(), location.span))
        .unwrap_or_default();
    let mut message = error.message;
    for label in error.labels {
        if label.location.is_none() {
            message.push('\n');
            message.push_str(&label.label);
        }
    }
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        message,
        ..Default::default()
    }
}

/// Converts character columns in the original text to UTF-16 LSP positions.
///
/// - Precondition: `span` refers to `source`.
/// - Complexity: O(n) time for the source prefix ending at the span; O(1) space.
pub(crate) fn source_range(source: &str, span: SourceSpan) -> Range {
    debug_assert!((span.start.line, span.start.column) <= (span.end.line, span.end.column));
    let position = |line: usize, column: usize| {
        let character = source
            .lines()
            .nth(line.saturating_sub(1))
            .unwrap_or("")
            .chars()
            .take(column)
            .map(char::len_utf16)
            .sum::<usize>();
        Position::new(line.saturating_sub(1) as u32, character as u32)
    };
    Range::new(
        position(span.start.line, span.start.column),
        position(span.end.line, span.end.column),
    )
}

/// Verifies syntax/type diagnostic content and original-source UTF-16 projection.
#[cfg(test)]
mod tests {
    use super::*;

    /// Converts parsed type and lexical error columns to exact UTF-16 offsets.
    #[test]
    fn source_diagnostics_preserve_unicode_locations() {
        for (source, token, length) in [
            (
                "// first line\n/* 😀 é */ sheet s { cell café: i32 = 1.0; }",
                "1.0",
                3,
            ),
            // Lexical failures identify a point rather than a complete token.
            (
                "// first line\n/* 😀 é */ sheet s { cell café: i32 = `; }",
                "`",
                0,
            ),
        ] {
            let diagnostics = diagnostics_for_source(source);
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            let offset = source.find(token).unwrap();
            let column = source[..offset]
                .rsplit('\n')
                .next()
                .unwrap()
                .encode_utf16()
                .count() as u32;
            assert_eq!(diagnostics[0].range.start, Position::new(1, column));
            assert_eq!(diagnostics[0].range.end, Position::new(1, column + length));
        }
    }

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

    /// Accepts a clean typed initializer.
    #[test]
    fn clean_sheet_has_no_diagnostics() {
        assert!(diagnostics_for_source("sheet s { cell x: i32 = 1; }").is_empty());
    }

    /// Checks local component definitions and named bindings in an unregistered file.
    #[test]
    fn source_modules_support_component_definitions_and_named_bindings() {
        let source = "sheet copy(input: i32, result: i32) {
            relationship { result := input; }
        }
        sheet app {
            source x: i32 = 1;
            cell y: i32;
            relationship copy(result: y, input: x);
        }";
        let diagnostics = diagnostics_for_source(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let diagnostics = diagnostics_for_source(&source.replace("result: y", "result: absent"));
        assert!(
            diagnostics
                .iter()
                .any(|error| error.message.contains("absent")),
            "{diagnostics:?}"
        );
    }

    /// Reports an unresolved import as a semantic error rather than rejecting `use`.
    #[test]
    fn source_modules_report_unresolved_imports() {
        let diagnostics =
            diagnostics_for_source("/* 😀 */ use missing::ordered as ord; sheet app {}");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("missing"),
            "{diagnostics:?}"
        );
        assert_eq!(
            diagnostics[0].range,
            Range::new(Position::new(0, 9), Position::new(0, 37))
        );
    }

    /// Checks named uses in a single root, including inactive and default branches.
    #[test]
    fn source_modules_check_single_root_component_uses() {
        for body in [
            "relationship missing();",
            "conditional false { true => { relationship missing(); } }",
            "conditional false { _ => { relationship missing(); } }",
        ] {
            let diagnostics = diagnostics_for_source(&format!("sheet app {{ {body} }}"));
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert!(
                diagnostics[0].message.contains("missing"),
                "{diagnostics:?}"
            );
        }
    }

    /// Retains every recovered syntax error after a valid import declaration.
    #[test]
    fn source_modules_recover_errors_after_imports() {
        let diagnostics = diagnostics_for_source(
            "use missing::ordered;
             sheet app { cell x unknown_syntax; cell y unknown_syntax; }",
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(diagnostics.iter().all(|error| error.range.start.line == 1));
    }

    /// Reports a mismatched initializer with error severity.
    #[test]
    fn type_mismatched_cell_initializer_is_a_diagnostic() {
        // Unsuffixed float literal defaults to f64, not the declared i32 — same fixture used by
        // adam-lang's own `check_sheet` test suite (adam-lang/src/typecheck.rs).
        let diags = diagnostics_for_source("sheet s { cell x: i32 = 1.0; }");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    /// Publishes a recovered declaration error.
    #[test]
    fn recovered_syntax_error_is_a_diagnostic() {
        // Malformed cell declaration; AdamAstParser recovers it as one `Sheet.errors` entry
        // (adam-lang/src/ast_parser.rs's `parse_malformed_cell_is_recorded_as_an_error_item`).
        assert_eq!(
            diagnostics_for_source("sheet s { cell x unknown_syntax }").len(),
            1
        );
    }

    /// Publishes every file-level recovery error for malformed module declarations.
    #[test]
    fn malformed_module_declarations_are_diagnostics() {
        assert_eq!(diagnostics_for_source("not a sheet at all").len(), 2);
    }

    /// Accepts a correctly typed placeholder filter.
    #[test]
    fn filter_clause_with_matching_types_has_no_diagnostics() {
        assert!(diagnostics_for_source("sheet s { cell a: i32 = 1 filter _; }").is_empty());
    }

    /// Rejects a filter that omits its placeholder.
    #[test]
    fn filter_clause_without_underscore_is_a_diagnostic() {
        let diags = diagnostics_for_source("sheet s { cell a: i32 = 1 filter 1; }");
        assert_eq!(diags.len(), 1);
    }
}
