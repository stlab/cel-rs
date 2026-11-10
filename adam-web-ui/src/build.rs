//! Parses, initializes, and validates adam-lang sheets before returning them.

use crate::labels::{
    Labels, Renderer, format_adam_error, format_module_diagnostic, format_parse_error,
    format_source_labels, format_validation_issue, labels_from_cell_names,
};
use adam_lang::{
    AdamParser, ModuleDiagnostic, ParsedSheet, SheetDiagnostics, SourceLabel, TypeRegistry,
};
use adam_rs::{CellId, Error};
use indexmap::IndexMap;
use std::collections::HashMap;

/// A runtime mutation error with independently owned source diagnostics.
///
/// The failed candidate is discarded and is never part of this value.
///
/// # Examples
///
/// See [`BuildError::render`] for formatting a runtime failure.
#[derive(Debug)]
pub struct RuntimeFailure {
    /// The typed error returned by a mutating sheet operation.
    pub error: Error,
    /// Source and provenance metadata captured before mutation.
    pub diagnostics: Box<SheetDiagnostics>,
    /// Requested display name for the anonymous root source.
    pub file_name: String,
}

/// A requirement that evaluated to false after successful propagation.
///
/// `key` identifies the affected root cell when it has a public name. Private
/// cells use their authored declaration label, or a cell ID when no source
/// label exists. `labels` retains source owners.
///
/// # Examples
///
/// See [`BuildError::Validation`] for the construction result that contains issues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationIssue {
    /// Stable public name or authored label of the affected cell.
    pub key: String,
    /// Explicit `@name` requirement label, when present.
    pub requirement: Option<String>,
    /// Source labels for the false requirement.
    pub labels: Vec<SourceLabel>,
    /// Human-readable explanation of the failed requirement.
    pub message: String,
}

/// A sheet that parsed, propagated successfully, and passed its requirements.
///
/// The key map starts with public root names. A host may add keys for private
/// cells after it configures its model.
///
/// # Examples
///
/// See [`build_sheet`] for constructing an initialized sheet.
pub struct BuiltSheet {
    /// The initialized live sheet and its source metadata.
    pub parsed: ParsedSheet,
    /// Display and write metadata for the live sheet.
    pub labels: Labels,
    /// Independently owned source and error-site metadata.
    pub diagnostics: SheetDiagnostics,
    /// Stable public root names mapped to their cell IDs.
    pub keys: IndexMap<String, CellId>,
    /// Root diagnostic display name retained for later host mutations.
    pub file_name: String,
}

/// A construction or restoration failure that never owns a candidate sheet.
///
/// # Examples
///
/// ```
/// let error = adam_web_ui::build_sheet("sheet s { cell x }", "input.adm2")
///     .err()
///     .expect("the module is invalid");
/// assert!(matches!(error, adam_web_ui::BuildError::Module(_)));
/// ```
#[derive(Debug)]
pub enum BuildError {
    /// A typed host replay or publication failure, without a candidate model.
    Recovery(Box<crate::binding::HostRejection>),
    /// A lexical or syntactic error with its source text and display name.
    Parse {
        /// The parser's typed error.
        error: Box<cel_parser::ParseError>,
        /// The complete input source.
        source: String,
        /// The display name used for the diagnostic header.
        name: String,
    },
    /// A module compilation or instantiation diagnostic.
    Module(ModuleDiagnostic),
    /// A fatal initial runtime propagation error.
    Runtime(Box<RuntimeFailure>),
    /// Requirements that evaluated to false after propagation.
    Validation {
        /// False requirements with their owning source labels.
        issues: Vec<ValidationIssue>,
        /// Source owners used to restrict the root display-name override.
        diagnostics: Box<SheetDiagnostics>,
        /// Requested display name for the anonymous root source.
        file_name: String,
    },
}

impl BuildError {
    /// Renders the failure without requiring access to a candidate sheet.
    ///
    /// - Complexity: O(rendered source and diagnostic bytes), excluding snippet
    ///   rendering and the underlying error formatter's costs.
    ///
    /// # Examples
    ///
    /// ```
    /// let result = adam_web_ui::build_sheet("sheet s { cell x }", "input.adm2");
    /// let error = result.err().expect("invalid module");
    /// assert!(error.render(&adam_web_ui::Renderer::plain()).contains("error"));
    /// ```
    pub fn render(&self, renderer: &Renderer) -> String {
        match self {
            Self::Recovery(rejection) => match rejection.as_ref() {
                crate::binding::HostRejection::Input(error) => error.to_string(),
                crate::binding::HostRejection::Binding(error) => error.to_string(),
                crate::binding::HostRejection::Validation(issues) => issues
                    .iter()
                    .map(|issue| {
                        format_source_labels(
                            &format!("{}: {}", issue.key, issue.message),
                            &issue.labels,
                            None,
                            renderer,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                crate::binding::HostRejection::Mutation(failure) => {
                    let source = failure.diagnostics.anonymous_source();
                    format_adam_error(
                        &failure.error,
                        &failure.diagnostics,
                        source.map_or("", adam_lang::SourceId::text),
                        &failure.file_name,
                        renderer,
                    )
                }
            },
            Self::Parse {
                error,
                source,
                name,
            } => format_parse_error(error, source, name, renderer),
            Self::Module(error) => format_module_diagnostic(error, renderer),
            Self::Runtime(failure) => {
                let source = failure.diagnostics.anonymous_source();
                format_adam_error(
                    &failure.error,
                    &failure.diagnostics,
                    source.map_or("", adam_lang::SourceId::text),
                    &failure.file_name,
                    renderer,
                )
            }
            Self::Validation {
                issues,
                diagnostics,
                file_name,
            } => issues
                .iter()
                .map(|issue| format_validation_issue(issue, diagnostics, file_name, renderer))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Builds a [`cel_parser::OpLookup`] with the CEL standard and Euclid libraries installed, so every
/// adam-lang source [`build_sheet`] parses has the same function set (`min`, `max`, `clamp`,
/// etc.) available.
///
/// Every source parsed through this function therefore has `cel-std` installed — a source
/// that deliberately relies on the standard library being *absent* cannot use this function
/// or [`build_sheet`]; construct an `AdamParser` directly with an empty `OpLookup` instead.
///
/// - Complexity: O(s²) in the number of installed signatures.
///
/// # Examples
/// ```
/// let lookup = adam_web_ui::build::op_lookup();
/// assert!(lookup.record_schema("Rect").is_some());
/// ```
pub fn op_lookup() -> cel_parser::OpLookup {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    lookup
}

/// Builds the matching scalar and Euclid cell-type environment.
///
/// - Complexity: O(t) in the number of registered types.
///
/// # Examples
/// ```
/// let types = adam_web_ui::build::type_registry();
/// assert!(types.get("Rect").is_some());
/// ```
pub fn type_registry() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    types
}

/// Parses and initializes a sheet, returning only a fully valid candidate.
///
/// `file_name` supplies the display name for diagnostics; it does not locate
/// the source text.
///
/// # Errors
///
/// Returns a parse, module, propagation, or requirement-validation failure.
/// Propagation errors retain their typed `adam_rs::Error` and independent
/// diagnostics. No error variant contains the failed candidate.
///
/// - Complexity: O(n) in source length plus propagation and validation work.
///
/// # Examples
///
/// ```
/// let built = adam_web_ui::build_sheet(
///     "sheet example { cell width: i64 = 10i64; }", "example.adm2",
/// ).unwrap();
/// assert!(built.keys.contains_key("width"));
/// ```
pub fn build_sheet(source: &str, file_name: &str) -> Result<BuiltSheet, BuildError> {
    let mut parser = AdamParser::new(type_registry(), op_lookup());
    let parsed = match parser.parse_str(source) {
        Ok(p) => p,
        Err(error) => {
            if let Some(module) = module_diagnostic(&error) {
                return Err(BuildError::Module(module.clone()));
            }
            return Err(BuildError::Parse {
                error: Box::new(error),
                source: source.to_owned(),
                name: file_name.to_owned(),
            });
        }
    };

    initialize_sheet_with_file_name(parsed, Some(file_name))
}

/// Finds an owning module diagnostic in a parser error's source chain.
///
/// - Complexity: O(h) time for source-chain depth h.
fn module_diagnostic(error: &cel_parser::ParseError) -> Option<&ModuleDiagnostic> {
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        if let Some(module) = error.downcast_ref() {
            return Some(module);
        }
        cause = error.source();
    }
    None
}

/// Propagates an explicitly selected parsed root and returns it only if valid.
///
/// Capture diagnostics before mutation. If propagation fails, discard the
/// unspecified candidate and return only its typed error and owned metadata.
///
/// # Errors
///
/// Returns a runtime or validation error when initial propagation fails or a
/// requirement evaluates to false.
///
/// - Complexity: O(m) metadata snapshot cost plus propagation and validation
///   work, where m is the retained metadata size.
///
/// # Examples
///
/// ```
/// use adam_lang::{AdamParser, TypeRegistry};
/// let parsed = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
///     .parse_str("sheet example { cell width: i64 = 10i64; }")
///     .unwrap();
/// let built = adam_web_ui::initialize_sheet(parsed).unwrap();
/// assert!(built.keys.contains_key("width"));
/// ```
pub fn initialize_sheet(parsed: ParsedSheet) -> Result<BuiltSheet, BuildError> {
    initialize_sheet_with_file_name(parsed, None)
}

/// Initializes a parsed sheet while retaining a caller-selected root display name.
///
/// - Complexity: O(m) metadata snapshot cost plus propagation and validation
///   work, where m is the retained metadata size.
fn initialize_sheet_with_file_name(
    mut parsed: ParsedSheet,
    requested_file_name: Option<&str>,
) -> Result<BuiltSheet, BuildError> {
    let diagnostics = parsed.diagnostics();
    let labels = labels_from_cell_names(&parsed.sheet, &parsed.cell_names);
    let keys = parsed
        .cell_names
        .iter()
        .map(|(name, (id, _))| (name.clone(), *id))
        .collect();
    match parsed.propagate() {
        Ok(()) => {
            let issues = validation_issues(&parsed);
            if !issues.is_empty() {
                return Err(BuildError::Validation {
                    issues,
                    file_name: display_file_name(&diagnostics, requested_file_name),
                    diagnostics: Box::new(diagnostics),
                });
            }
            parsed.clear_changed();
            Ok(BuiltSheet {
                file_name: display_file_name(&diagnostics, requested_file_name),
                parsed,
                labels,
                diagnostics,
                keys,
            })
        }
        Err(error) => Err(BuildError::Runtime(Box::new(RuntimeFailure {
            error,
            file_name: display_file_name(&diagnostics, requested_file_name),
            diagnostics: Box::new(diagnostics),
        }))),
    }
}

/// Selects an explicit root display name or the parser's retained source name.
///
/// - Complexity: O(n) in the selected file-name byte length.
fn display_file_name(diagnostics: &SheetDiagnostics, requested_file_name: Option<&str>) -> String {
    requested_file_name
        .map(str::to_owned)
        .or_else(|| {
            diagnostics
                .anonymous_source()
                .map(|source| source.file_name().to_owned())
        })
        .unwrap_or_default()
}

/// Collects every false requirement from the successfully propagated candidate.
///
/// - Complexity: O(c + r) time and O(v) output space for cells c, requirements r,
///   and violations v.
pub(crate) fn validation_issues(parsed: &ParsedSheet) -> Vec<ValidationIssue> {
    let names: HashMap<_, _> = parsed
        .cell_names
        .iter()
        .map(|(name, (id, _))| (*id, name.as_str()))
        .collect();
    let mut issues = Vec::new();
    for cell in parsed.cells() {
        for requirement_id in parsed.violated_requirements(cell) {
            let requirement = parsed.requirement_names.get(&requirement_id).cloned();
            let key = names
                .get(&cell)
                .map(|name| (*name).to_owned())
                .or_else(|| {
                    parsed
                        .cell_origins
                        .get(&cell)
                        .and_then(|origins| origins.first())
                        .map(|origin| origin.label.label.clone())
                })
                .unwrap_or_else(|| format!("{cell:?}"));
            let message = match requirement.as_deref() {
                Some(name) => format!("requirement `{name}` evaluated to false"),
                None => "requirement evaluated to false".to_string(),
            };
            let labels = parsed
                .requirement_labels
                .get(&requirement_id)
                .cloned()
                .into_iter()
                .collect();
            issues.push(ValidationIssue {
                key,
                requirement,
                labels,
                message,
            });
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rejects a candidate when its first propagation fails.
    #[test]
    fn construction_rejects_initial_runtime_failure() {
        assert!(matches!(
            build_sheet("sheet s { out x: i64 := 1i64 / 0i64; }", "s.adm2"),
            Err(BuildError::Runtime(_))
        ));
    }

    /// Runtime diagnostics use the requested root display name.
    #[test]
    fn runtime_diagnostic_uses_requested_file_name() {
        let error = build_sheet(
            "sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }",
            "s.adm2",
        )
        .err()
        .expect("division by zero rejects construction");

        let rendered = error.render(&Renderer::plain());
        assert!(rendered.contains("s.adm2"), "{rendered}");
    }

    /// Rejects a candidate when a requirement is false after propagation.
    #[test]
    fn construction_rejects_initial_requirement_violation() {
        let result = build_sheet(
            "sheet s { cell x: i64 = -1i64 require { @positive x > 0i64; }; }",
            "s.adm2",
        );
        let Err(error) = result else {
            panic!("false initial requirement must reject construction");
        };
        let BuildError::Validation { issues, .. } = &error else {
            panic!("false initial requirement must be Validation");
        };
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].key, "x");
        assert_eq!(issues[0].requirement.as_deref(), Some("positive"));
        assert!(
            issues[0].labels.iter().any(|label| {
                label
                    .location
                    .as_ref()
                    .is_some_and(|location| location.source.text().contains("@positive"))
            }),
            "validation issue should retain its authored requirement source"
        );
        assert!(
            error
                .render(&Renderer::plain())
                .contains("requirement `positive` evaluated to false")
        );
    }

    /// Validation diagnostics use the requested root display name.
    #[test]
    fn validation_diagnostic_uses_requested_file_name() {
        let error = build_sheet(
            "sheet s { cell x: i64 = -1i64 require { @positive x > 0i64; }; }",
            "s.adm2",
        )
        .err()
        .expect("false requirement rejects construction");

        let rendered = error.render(&Renderer::plain());
        assert!(rendered.contains("s.adm2"), "{rendered}");
    }

    /// The opted-in host builds registered fields, members, and typed closures.
    #[test]
    fn geometry_sheet_builds_in_opted_in_host() {
        let built = build_sheet(
            "sheet s { out x := (|p: Point2D| p.x)(Point2D { y: 2.0, x: 1.0 }); out n := min(3.0, 4.0); }",
            "geometry.adm2",
        )
        .unwrap();
        assert_eq!(
            *built
                .parsed
                .read::<f64>(built.parsed.cell_names["x"].0)
                .unwrap(),
            1.0
        );
        for actual in [
            include_str!("../../cel-euclid/examples/crop_straighten.adm2"),
            include_str!("../../cel-euclid/examples/pan_zoom.adm2"),
        ] {
            build_sheet(actual, "geometry.adm2").unwrap();
        }
    }

    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relationship {
                c := a * b;
                a := c / b;
                b := c / a;
            }
        }
    "#;

    #[test]
    /// Valid source builds and propagates without a diagnostic.
    fn build_sheet_valid_source_succeeds_with_no_error() {
        let built = build_sheet(VALID_SOURCE, "test.adm2").unwrap();
        assert!(built.keys.contains_key("a"));
    }

    #[test]
    /// Module failures return typed errors that render a source-located diagnostic.
    fn build_sheet_module_error_has_no_sheet_and_formatted_message() {
        let error = build_sheet("sheet s { cell x }", "test.adm2")
            .err()
            .expect("expected module error");
        assert!(matches!(error, BuildError::Module(_)), "{error:?}");
        let msg = error.render(&Renderer::styled());
        assert!(msg.contains("error"), "{msg}");
    }

    #[test]
    /// Propagation failures return no candidate and retain the typed error.
    fn build_sheet_runtime_error_rejects_candidate() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }";
        let error = build_sheet(source, "test.adm2")
            .err()
            .expect("expected runtime error");
        let BuildError::Runtime(failure) = error else {
            panic!("expected runtime failure");
        };
        assert!(matches!(failure.error, Error::MethodFailed { .. }));
        assert!(!failure.diagnostics.locate_error(&failure.error).is_empty());
    }

    #[test]
    /// Plain module diagnostics contain no terminal escape sequences.
    fn build_sheet_plain_renderer_module_error_has_no_ansi_escape_codes() {
        let msg = build_sheet("sheet s { cell x }", "test.adm2")
            .err()
            .expect("expected parse error")
            .render(&Renderer::plain());
        assert!(
            !msg.contains('\u{1b}'),
            "expected no ANSI escapes, got: {msg}"
        );
    }

    #[test]
    /// Plain runtime diagnostics contain no terminal escape sequences.
    fn build_sheet_plain_renderer_runtime_error_has_no_ansi_escape_codes() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }";
        let msg = build_sheet(source, "test.adm2")
            .err()
            .expect("expected propagation error")
            .render(&Renderer::plain());
        assert!(
            !msg.contains('\u{1b}'),
            "expected no ANSI escapes, got: {msg}"
        );
    }

    #[test]
    /// Cycle diagnostics identify both relationships' binding spans.
    fn build_sheet_cycle_names_both_relationships_bindings_in_the_backtrace() {
        // Two relationships forming an algebraic loop with no external source for
        // either cell: `x` needs `y` and `y` needs `x`, so `propagate` can't pick a
        // valid method assignment and returns a multi-span `Error::Cycle` backtrace
        // naming both bindings.
        let source = "sheet s { cell x: i32 = 0; cell y: i32 = 0; \
            relationship { x := y + 1i32; } relationship { y := x + 1i32; } }";
        let msg = build_sheet(source, "test.adm2")
            .err()
            .expect("expected cycle error")
            .render(&Renderer::plain());
        assert!(msg.contains("x :=") && msg.contains("y :="), "{msg}");
    }
}
