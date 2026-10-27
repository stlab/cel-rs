//! Cell display/write metadata and diagnostic formatting for a live [`adam_rs::Sheet`].
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] keys, driving [`crate::SheetInspector`]'s rendering.
//! [`format_adam_error`] formats an [`adam_rs::Error`] as a rustc-style diagnostic when
//! possible.

use adam_lang::type_registry::TypeShape;
use adam_rs::{CellId, Error, Sheet};
pub use annotate_snippets::Renderer;
use cel_parser::{FormatRustcStyle, SpanContext, SpanLabel};
use indexmap::IndexMap;
use std::any::TypeId;

/// Type-erased write closure: parses a string and writes it to a cell.
pub type WriteStrFn = Box<dyn Fn(&mut Sheet, &str) -> Result<(), Error>>;

/// Display and write metadata for a single cell.
pub struct CellMeta {
    /// Human-readable cell name shown in the inspector.
    pub label: String,
    /// `true` if the cell holds a `bool`, so [`crate::SheetInspector`] can render it as a
    /// checkbox instead of a text field.
    pub is_bool: bool,
    /// `true` if the cell holds one of the 14 numeric primitive types, so
    /// [`crate::SheetInspector`] can render it with [`crate::spectrum::SpNumberfield`]
    /// instead of a plain text field.
    pub is_numeric: bool,
    /// `true` if the cell holds one of the 12 integer primitive types (as opposed to
    /// `f32`/`f64`), so [`crate::SheetInspector`] can set its numeric widget's `step` to `1` —
    /// [`crate::spectrum::SpNumberfield`]'s, or, for a range-filtered cell,
    /// [`crate::spectrum::SpSlider`]'s — since an integer cell has no fractional steps to land
    /// on.
    pub is_integer: bool,
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses `s` and writes the result to the cell; returns `Err` on parse failure or type
    /// mismatch. May also always return `Err` for a cell type with no write support yet (e.g.
    /// compound values — see [`Labels::add_compound_cell`]).
    pub write_str: WriteStrFn,
    /// Live slider bounds, present only for a numeric cell whose filter is a
    /// [`adam_rs::FilterKind::Range`] — recomputed from the filter's current argument values on
    /// every call, so a range driven by other cells or relationships stays live. Cast to `f64`
    /// for display, matching [`format_rounded`]'s existing all-numeric-types-as-`f64` convention.
    #[allow(clippy::type_complexity)]
    pub range: Option<Box<dyn Fn(&Sheet) -> (f64, f64)>>,
}

/// Associates human-readable labels and type-erased closures with stable sheet IDs.
pub struct Labels {
    /// Cells in insertion order (preserves display ordering).
    pub cells: IndexMap<CellId, CellMeta>,
}

impl Labels {
    /// Creates an empty label set.
    pub fn new() -> Self {
        Self {
            cells: IndexMap::new(),
        }
    }

    /// Registers display metadata for a cell of type `T`.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with.
    /// - Precondition: `T` matches the type registered with `Sheet::add_cell` for `id`.
    pub fn add_cell<T>(&mut self, id: CellId, label: &str)
    where
        T: std::any::Any + std::fmt::Display + std::str::FromStr + 'static,
        T::Err: std::fmt::Display,
    {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                is_bool: TypeId::of::<T>() == TypeId::of::<bool>(),
                is_numeric: false,
                is_integer: false,
                display: Box::new(move |sheet| {
                    sheet
                        .read::<T>(id)
                        .map(|v| format!("{}", v))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(move |sheet, s| {
                    let value = s.parse::<T>().map_err(|e| Error::MethodFailed {
                        error: anyhow::anyhow!("parse error: {}", e),
                        sites: vec![],
                    })?;
                    sheet.write(id, value)
                }),
                range: None,
            },
        );
    }

    /// Registers display-only metadata for a canonical compound cell of any shape.
    ///
    /// `write_str` always returns `Err` — no compound editor exists yet (tracked as a
    /// follow-up: see the "Support editing tuple-typed cells in `begin`" GitHub issue). The
    /// field still participates fully in [`crate::SheetInspector`]'s existing
    /// invalid/warning/disabled machinery, since that's entirely keyed on `CellId`, not on any
    /// per-type behavior.
    ///
    /// Missing storage or formatting capability displays `?`; unsupported writes
    /// always return an explicit error without mutating the sheet.
    ///
    /// - Complexity: Preparation O(1); display O(n + v + s) for n type nodes,
    ///   v values and s output bytes, excluding leaf formatter costs.
    ///
    /// # Examples
    /// ```
    /// let sheet = adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { cell xs: [i32]; }").unwrap();
    /// let id = sheet.cell_names["xs"].0;
    /// let mut labels = adam_web_ui::Labels::new();
    /// labels.add_compound_cell(&sheet, id, "xs");
    /// assert_eq!((labels.cells[&id].display)(&sheet), "[]");
    /// ```
    pub fn add_compound_cell(&mut self, sheet: &Sheet, id: CellId, label: &str) {
        let formatter = adam_lang::prepare_compound_cell_formatter(sheet, id);
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                is_bool: false,
                is_numeric: false,
                is_integer: false,
                display: Box::new(move |sheet| {
                    formatter
                        .as_ref()
                        .ok()
                        .and_then(|format| format(sheet).ok())
                        .unwrap_or_else(|| "?".to_owned())
                }),
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed {
                        error: anyhow::anyhow!("editing compound cells is not yet supported"),
                        sites: vec![],
                    })
                }),
                range: None,
            },
        );
    }
}

impl Default for Labels {
    /// Returns `Labels::new()`.
    fn default() -> Self {
        Self::new()
    }
}

/// Formats a floating-point value for display, rounded to 2 decimal places with
/// trailing zeros (and a bare trailing decimal point) trimmed.
///
/// Used in place of plain `Display` for `f32`/`f64` cells so inspector fields show `86.67`
/// and `300` rather than `86.666666666667` and `300.0`. Not applied to other cell types:
/// precision has no meaningful effect on integers, and it would truncate `String` values
/// outright.
///
/// # Examples
///
/// ```text
/// format_rounded(86.666666666667) == "86.67"
/// format_rounded(300.0)            == "300"
/// format_rounded(-0.001)           == "0"
/// ```
pub fn format_rounded(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Converts a filter-recognized numeric primitive to `f64` for display — the same "every numeric
/// type displays as `f64`" convention [`format_rounded`] already documents. Implemented for
/// exactly the 14 primitives `TypeRegistry::range_entry` recognizes range support for; `i64`,
/// `u64`, `i128`, `u128`, `usize`, and `isize` lose precision beyond 2^53, identical to
/// `labels_from_cell_names`'s existing `try_float_ty!`-driven display path for those types.
trait ToF64Display {
    fn to_f64_display(&self) -> f64;
}

macro_rules! impl_to_f64_display {
    ($($T:ty),*) => {
        $(impl ToF64Display for $T {
            fn to_f64_display(&self) -> f64 {
                *self as f64
            }
        })*
    };
}
impl_to_f64_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Builds a [`Labels`] from an adam-lang-style declaration-ordered cell name map.
///
/// Matches each scalar cell's `TypeId` against the built-in primitive types
/// `adam_lang::TypeRegistry::new()` registers. Tuple and array cells
/// appear with a Debug-formatted, display-only entry via
/// [`Labels::add_compound_cell`]. Cells whose `TypeId` is none of the built-in
/// primitives are silently skipped, so they simply won't appear in the sidebar.
/// Callable signatures are skipped because no callable Inspector exists.
///
/// - Complexity: O(n) in the number of cells.
pub fn labels_from_cell_names(
    sheet: &Sheet,
    cell_names: &IndexMap<String, (CellId, TypeShape)>,
) -> Labels {
    let mut labels = Labels::new();
    for (name, (id, shape)) in cell_names {
        let id = *id;
        let type_id = match shape {
            TypeShape::Named(type_id) => *type_id,
            TypeShape::Tuple(_) | TypeShape::Array(_) => {
                labels.add_compound_cell(sheet, id, name);
                continue;
            }
            TypeShape::Callable(_, _) => continue,
        };
        macro_rules! try_numeric_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    mark_numeric::<$T>(&mut labels, sheet, id, true);
                    continue;
                }
            };
        }
        try_numeric_ty!(i8);
        try_numeric_ty!(i16);
        try_numeric_ty!(i32);
        try_numeric_ty!(i64);
        try_numeric_ty!(i128);
        try_numeric_ty!(isize);
        try_numeric_ty!(u8);
        try_numeric_ty!(u16);
        try_numeric_ty!(u32);
        try_numeric_ty!(u64);
        try_numeric_ty!(u128);
        try_numeric_ty!(usize);

        macro_rules! try_numeric_float_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    if let Some(meta) = labels.cells.get_mut(&id) {
                        meta.display = Box::new(move |sheet| {
                            sheet
                                .read::<$T>(id)
                                .map(|v| format_rounded(*v as f64))
                                .unwrap_or_else(|_| "?".to_owned())
                        });
                    }
                    mark_numeric::<$T>(&mut labels, sheet, id, false);
                    continue;
                }
            };
        }
        try_numeric_float_ty!(f32);
        try_numeric_float_ty!(f64);

        macro_rules! try_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    continue;
                }
            };
        }
        try_ty!(bool);
        try_ty!(String);
    }
    labels
}

/// Marks `id`'s `CellMeta` as numeric (and, per `is_integer`, as an integer type) and, if
/// `sheet.filter_kind(id)` is a range clamp, populates its live-range closure.
fn mark_numeric<T: std::any::Any + Clone + ToF64Display>(
    labels: &mut Labels,
    sheet: &Sheet,
    id: CellId,
    is_integer: bool,
) {
    let Some(meta) = labels.cells.get_mut(&id) else {
        return;
    };
    meta.is_numeric = true;
    meta.is_integer = is_integer;
    if matches!(
        sheet.filter_kind(id),
        Some(adam_rs::FilterKind::Range { .. })
    ) {
        meta.range = Some(Box::new(move |sheet: &Sheet| {
            sheet
                .filter_range::<T>(id)
                .map(|(lo, hi)| (lo.to_f64_display(), hi.to_f64_display()))
                .unwrap_or((0.0, 0.0))
        }));
    }
}

/// Formats an [`Error`] as a rustc-style diagnostic when possible.
///
/// Resolves native sites through `parsed`, groups labels by owning source identity
/// and renders definition files followed by invocation context. Inner CEL spans use
/// their definition text, never the caller's text. Source-free labels remain visible.
/// `file_name` overrides only the anonymous `AdamParser` input's display name.
/// For legacy anonymous errors with no native site, `source` supplies the CEL text.
///
/// - Complexity: Provenance resolution plus O(n + l) source bytes and labels rendered.
///
/// # Examples
/// ```
/// let parsed = adam_lang::ParsedSheet::new();
/// let text = adam_web_ui::format_adam_error(&adam_rs::Error::InvalidId,
///     &parsed, "", "input.adm2", &adam_web_ui::Renderer::plain());
/// assert_eq!(text, "invalid cell or relationship id");
/// ```
pub fn format_adam_error(
    e: &Error,
    parsed: &adam_lang::ParsedSheet,
    source: &str,
    file_name: &str,
    renderer: &Renderer,
) -> String {
    let located = parsed.locate_error(e);
    if located.is_empty()
        && e.sites().is_empty()
        && parsed.anonymous_source().is_some()
        && let Error::MethodFailed { error, .. } = e
        && error.downcast_ref::<SpanContext>().is_some()
    {
        return error.format_rustc_style(source, file_name, 1, renderer);
    }
    format_source_labels(
        &runtime_message(e),
        &located,
        parsed.anonymous_source().map(|source| (source, file_name)),
        renderer,
    )
}

/// Returns the operation's message without replacing or consuming its typed error.
fn runtime_message(error: &Error) -> String {
    if let Error::MethodFailed { error, .. } = error
        && error.downcast_ref::<SpanContext>().is_some()
    {
        return error
            .source()
            .map_or_else(|| error.to_string(), ToString::to_string);
    }
    error.to_string()
}

/// Formats a module diagnostic against its retained source owners.
///
/// Native labels remain visible without invented locations.
/// - Complexity: O(n + l) source and label bytes, excluding snippet rendering.
///
/// # Examples
/// ```
/// let error = adam_lang::ModuleDiagnostic {
///     message: "missing root".into(), labels: vec![], cause: None,
/// };
/// assert_eq!(adam_web_ui::format_module_diagnostic(&error,
///     &adam_web_ui::Renderer::plain()), "missing root");
/// ```
pub fn format_module_diagnostic(
    error: &adam_lang::ModuleDiagnostic,
    renderer: &Renderer,
) -> String {
    format_source_labels(&diagnostic_message(error), &error.labels, None, renderer)
}

/// Formats the convenience parser's owning diagnostic, retaining the caller's file name.
///
/// Legacy lexical failures without owning labels use the parser's original formatter.
/// This adapter borrows the error and preserves every concrete source cause.
/// - Complexity: O(h + n + l) cause depth h, source bytes n and label bytes l.
///
/// # Examples
/// ```
/// let error = adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(),
///     cel_parser::OpLookup::new()).parse_str("sheet {}").unwrap_err();
/// let text = adam_web_ui::format_parse_error(&error, "sheet {}", "file.adm2",
///     &adam_web_ui::Renderer::plain());
/// assert!(!text.is_empty());
/// ```
pub fn format_parse_error(
    error: &cel_parser::ParseError,
    source: &str,
    file_name: &str,
    renderer: &Renderer,
) -> String {
    let diagnostic =
        std::iter::successors(std::error::Error::source(error), |error| error.source())
            .find_map(|cause| cause.downcast_ref::<adam_lang::ModuleDiagnostic>());
    if let Some(diagnostic) = diagnostic
        && diagnostic
            .labels
            .iter()
            .any(|label| label.location.is_some())
    {
        let owner = diagnostic
            .labels
            .iter()
            .find_map(|label| label.location.as_ref().map(|location| &location.source));
        return format_source_labels(
            &diagnostic_message(diagnostic),
            &diagnostic.labels,
            owner.map(|owner| (owner, file_name)),
            renderer,
        );
    }
    error.format_rustc_style(source, file_name, 1, renderer)
}

/// Includes the concrete innermost cause when a span context hides its message.
///
/// - Complexity: O(h + m) cause depth h and formatted message bytes m.
fn diagnostic_message(error: &adam_lang::ModuleDiagnostic) -> String {
    let mut message = error.message.clone();
    if let Some(cause) =
        std::iter::successors(std::error::Error::source(error), |error| error.source()).last()
    {
        let detail = cause.to_string();
        if !message.contains(&detail) {
            message.push_str(": ");
            message.push_str(&detail);
        }
    }
    message
}

/// Renders owning labels in first-source order, retaining span-free context.
///
/// Only `display_source`'s exact owner receives the supplied display name.
/// Empty labels return the message unchanged.
/// - Complexity: Expected O(n + l) time and O(n + l) space for source bytes n
///   and label bytes l, excluding the existing snippet renderer.
fn format_source_labels(
    message: &str,
    labels: &[adam_lang::SourceLabel],
    display_source: Option<(&adam_lang::SourceId, &str)>,
    renderer: &Renderer,
) -> String {
    let mut groups: IndexMap<&adam_lang::SourceId, Vec<SpanLabel>> = IndexMap::new();
    for label in labels {
        if let Some(location) = &label.location {
            groups.entry(&location.source).or_default().push(SpanLabel {
                span: location.span,
                label: label.label.clone(),
            });
        }
    }
    let mut result = String::new();
    for (source, spans) in groups {
        let file_name = display_source
            .filter(|(owner, _)| *owner == source)
            .map_or_else(|| source.file_name(), |(_, name)| name);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&cel_parser::format_multi_span(
            message,
            &spans,
            source.text(),
            file_name,
            1,
            renderer,
        ));
    }
    if result.is_empty() {
        result.push_str(message);
    }
    for label in labels.iter().filter(|label| label.location.is_none()) {
        result.push_str("\n= context: ");
        result.push_str(&label.label);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_rs::Sheet as AdamSheet;

    /// Renders each source owner separately and keeps span-free context visible.
    #[test]
    fn labels_render_owned_files_and_native_context() {
        let mut sources = adam_lang::SourceMap::new();
        let definition = sources.insert("definition.adm2", "a / b");
        let invocation = sources.insert("caller.adm2", "relationship divide(a: a, b: b);");
        let labels = vec![
            adam_lang::SourceLabel {
                location: Some(adam_lang::SourceLocation {
                    source: definition,
                    span: cel_parser::SourceSpan::new(1, 2, 1, 3),
                }),
                label: "division failed".into(),
            },
            adam_lang::SourceLabel {
                location: Some(adam_lang::SourceLocation {
                    source: invocation,
                    span: cel_parser::SourceSpan::new(1, 0, 1, 31),
                }),
                label: "component use".into(),
            },
            adam_lang::SourceLabel {
                location: None,
                label: "native definition".into(),
            },
        ];
        let rendered = format_source_labels("division by zero", &labels, None, &Renderer::plain());
        for expected in [
            "definition.adm2:1:3",
            "caller.adm2:1:1",
            "a / b",
            "component use",
            "native definition",
        ] {
            assert!(rendered.contains(expected), "{rendered}");
        }
        assert!(!rendered.contains('\u{1b}'));
    }

    /// Preserves an anonymous root's display name, exact division subspan and original cause.
    #[test]
    fn labels_render_construction_cel_error_with_original_cause() {
        let source = "sheet app { cell a: i32 = 1 + 4 / 0; }";
        let error =
            adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
                .parse_str(source)
                .unwrap_err();
        let rendered = format_parse_error(&error, source, "root.adm2", &Renderer::plain());
        assert!(rendered.contains("division by zero"), "{rendered}");
        assert!(rendered.contains("root.adm2:1:31"), "{rendered}");
        assert!(!rendered.contains("<input>"), "{rendered}");
        let cause = std::error::Error::source(&error).unwrap();
        assert!(cause.is::<adam_lang::ModuleDiagnostic>());
    }

    /// Keeps independent identical sources separate and overrides only the requested owner.
    #[test]
    fn labels_group_by_identity_and_keep_unlocated_and_empty_messages() {
        let mut sources = adam_lang::SourceMap::new();
        let a = sources.insert("same.adm2", "x");
        let b = sources.insert("same.adm2", "x");
        let labels: Vec<_> = [&a, &b]
            .into_iter()
            .map(|source| adam_lang::SourceLabel {
                location: Some(adam_lang::SourceLocation {
                    source: source.clone(),
                    span: cel_parser::SourceSpan::new(1, 0, 1, 1),
                }),
                label: "definition".into(),
            })
            .collect();
        let rendered = format_source_labels(
            "boom",
            &labels,
            Some((&a, "caller.adm2")),
            &Renderer::plain(),
        );
        assert!(rendered.contains("caller.adm2"), "{rendered}");
        assert!(rendered.contains("same.adm2"), "{rendered}");
        assert_eq!(
            format_source_labels("boom", &[], None, &Renderer::plain()),
            "boom"
        );
        let diagnostic = adam_lang::ModuleDiagnostic {
            message: "boom".into(),
            labels: vec![adam_lang::SourceLabel {
                location: None,
                label: "native".into(),
            }],
            cause: None,
        };
        assert_eq!(
            format_module_diagnostic(&diagnostic, &Renderer::plain()),
            "boom\n= context: native"
        );
    }

    /// Uses imported definition text and the failing second invocation, never caller text.
    #[test]
    fn labels_render_imported_runtime_definition_and_actual_use() {
        let mut modules = adam_lang::ModuleSet::new();
        modules
            .insert_source(
                "library".parse().unwrap(),
                "library.adm2",
                "sheet divide(a: i32, b: i32) {\nrelationship { b := 1 + 4 / a; }\n}",
            )
            .unwrap();
        modules
            .insert_source(
                "app".parse().unwrap(),
                "app.adm2",
                "use library::divide;
sheet app {
source good: i32 = 2; source bad: i32 = 0; cell first: i32; cell second: i32;
relationship divide(a: good, b: first);
relationship divide(a: bad, b: second);
}",
            )
            .unwrap();
        let compiled = adam_lang::ModuleCompiler::new(
            adam_lang::TypeRegistry::new(),
            cel_parser::OpLookup::new(),
        )
        .compile(modules)
        .unwrap();
        let mut parsed = compiled
            .instantiate_root(&"app".parse().unwrap(), "app")
            .unwrap();
        drop(compiled);
        let error = parsed.propagate().unwrap_err();
        let rendered = format_adam_error(
            &error,
            &parsed,
            "WRONG CALLER TEXT",
            "wrong.adm2",
            &Renderer::plain(),
        );
        for expected in [
            "division by zero",
            "library.adm2:2:25",
            "app.adm2:5:1",
            "b := 1 + 4 / a",
            "b: second",
        ] {
            assert!(rendered.contains(expected), "{rendered}");
        }
        assert!(!rendered.contains("wrong.adm2"), "{rendered}");
        assert!(!rendered.contains("WRONG CALLER TEXT"), "{rendered}");
    }

    /// Parses `src` into a `ParsedSheet` via the standard adam-lang pipeline, for tests that
    /// need a real (rather than hand-built) span table to resolve `Error` sites against.
    fn parse(src: &str) -> adam_lang::ParsedSheet {
        use adam_lang::{AdamParser, TypeRegistry};
        use cel_parser::OpLookup;
        AdamParser::new(TypeRegistry::new(), OpLookup::new())
            .parse_str(src)
            .unwrap()
    }

    #[test]
    fn format_adam_error_invalid_id_falls_back_to_display() {
        let parsed = parse("sheet s { cell a: i32 = 0; }");
        let msg = format_adam_error(
            &Error::InvalidId,
            &parsed,
            "sheet s { cell a: i32 = 0; }",
            "test.adm2",
            &Renderer::styled(),
        );
        assert_eq!(msg, "invalid cell or relationship id");
    }

    #[test]
    fn format_adam_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let parsed = parse("sheet s { cell a: i32 = 0; }");
        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed {
            error: inner,
            sites: vec![],
        };

        let msg = format_adam_error(&err, &parsed, source, "test.adm2", &Renderer::styled());

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
    }

    #[test]
    fn format_adam_error_plain_renderer_has_no_ansi_escape_codes() {
        use cel_parser::{SourceSpan, SpanContext};

        let parsed = parse("sheet s { cell a: i32 = 0; }");
        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed {
            error: inner,
            sites: vec![],
        };

        let msg = format_adam_error(&err, &parsed, source, "test.adm2", &Renderer::plain());

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(
            !msg.contains('\u{1b}'),
            "expected no ANSI escapes, got: {msg}"
        );
    }

    #[test]
    fn format_adam_error_method_failed_falls_back_to_a_single_located_span_without_a_span_context()
    {
        use adam_rs::ErrorSite;

        let source =
            "sheet s {\n    cell a: i32;\n    relationship {\n        a := 1i32;\n    }\n}";
        let parsed = parse(source);
        let &(rel_id, idx) = parsed.method_spans.keys().next().expect("one relationship");

        let err = Error::MethodFailed {
            error: anyhow::anyhow!("boom"),
            sites: vec![ErrorSite::Method(rel_id, idx)],
        };

        let msg = format_adam_error(&err, &parsed, source, "test.adm2", &Renderer::plain());

        assert!(msg.contains("boom"), "{msg}");
        assert!(msg.contains("a := 1i32;"), "{msg}");
    }

    #[test]
    fn format_adam_error_cycle_renders_a_multi_span_backtrace() {
        use adam_lang::{AdamParser, TypeRegistry};
        use cel_parser::OpLookup;
        let src = "sheet s { cell x: i32 = 0; cell y: i32 = 0; relationship { x := y + 1i32; } relationship { y := x + 1i32; } }";
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser.parse_str(src).unwrap();
        let err = parsed.propagate().unwrap_err();
        let msg = format_adam_error(&err, &parsed, src, "t.adm2", &Renderer::plain());
        assert!(msg.contains("cycle"), "{msg}");
        // both relationship bindings referenced
        assert!(msg.contains("x :=") && msg.contains("y :="), "{msg}");
    }

    #[test]
    fn format_rounded_trims_trailing_zeros_and_point() {
        assert_eq!(format_rounded(86.666666666667), "86.67");
        assert_eq!(format_rounded(300.0), "300");
        assert_eq!(format_rounded(2.5), "2.5");
        assert_eq!(format_rounded(0.0), "0");
    }

    #[test]
    fn format_rounded_negative_zero_has_no_minus_sign() {
        assert_eq!(format_rounded(-0.0), "0");
        assert_eq!(format_rounded(-0.001), "0");
    }

    #[test]
    fn labels_from_cell_names_marks_float_cells_as_numeric_but_not_integer() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(2.0_f64);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert!(labels.cells[&a].is_numeric);
        assert!(!labels.cells[&a].is_integer);
    }

    #[test]
    fn labels_from_cell_names_rounds_float_display_to_two_decimals() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(86.666666666667_f64);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert_eq!((labels.cells[&a].display)(&sheet), "86.67");
    }

    #[test]
    fn labels_from_cell_names_builds_entries_for_supported_types() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(2.0_f64);
        let b = sheet.add_cell(3_i32);
        let c = sheet.add_cell(true);
        let d = sheet.add_cell("hi".to_string());

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));
        cell_names.insert("b".to_string(), (b, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("c".to_string(), (c, TypeShape::Named(TypeId::of::<bool>())));
        cell_names.insert(
            "d".to_string(),
            (d, TypeShape::Named(TypeId::of::<String>())),
        );

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert_eq!(labels.cells.len(), 4);
        assert_eq!((labels.cells[&a].display)(&sheet), "2");
        assert_eq!((labels.cells[&b].display)(&sheet), "3");
        assert_eq!((labels.cells[&c].display)(&sheet), "true");
        assert_eq!((labels.cells[&d].display)(&sheet), "hi");
        assert!(!labels.cells[&a].is_bool);
        assert!(!labels.cells[&b].is_bool);
        assert!(labels.cells[&c].is_bool);
        assert!(!labels.cells[&d].is_bool);
    }

    #[test]
    fn labels_from_cell_names_includes_tuple_typed_cells() {
        let sheet = tuple_sheet();
        let pair = sheet.cell_names["pair"].0;

        let mut cell_names = IndexMap::new();
        cell_names.insert(
            "pair".to_string(),
            (
                pair,
                TypeShape::Tuple(vec![
                    TypeShape::Named(TypeId::of::<i32>()),
                    TypeShape::Named(TypeId::of::<f64>()),
                ]),
            ),
        );

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert_eq!(labels.cells.len(), 1);
        assert_eq!((labels.cells[&pair].display)(&sheet), "(3, 4.5)");
        assert!(!labels.cells[&pair].is_bool);
    }

    /// Compound labels display nested tuples, arrays, empty arrays, and source unit.
    #[test]
    fn labels_display_compound_values_and_reject_edits() {
        let mut sheet =
            adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
                .parse_str(
                    "sheet s {
                        cell xs: [(i32, bool)] = [(3, true), (4, false)];
                        cell empty: [i32];
                        cell nested: ([(i32, bool)], [i32]) = ([(7, true)], []: [i32]);
                        cell unit: ();
                        cell scalar: i32 = 5;
                    }",
                )
                .unwrap();
        let labels = labels_from_cell_names(&sheet, &sheet.cell_names);
        assert_eq!(labels.cells.len(), 5);
        for (name, expected) in [
            ("xs", "[(3, true), (4, false)]"),
            ("empty", "[]"),
            ("nested", "([(7, true)], [])"),
            ("unit", "()"),
        ] {
            let meta = &labels.cells[&sheet.cell_names[name].0];
            assert_eq!((meta.display)(&sheet), expected);
            assert!(!meta.is_bool && !meta.is_numeric && !meta.is_integer);
            let error = (meta.write_str)(&mut sheet, "[]").unwrap_err();
            assert!(error.to_string().contains("not yet supported"));
            assert_eq!((meta.display)(&sheet), expected);
        }
        let scalar = &labels.cells[&sheet.cell_names["scalar"].0];
        assert_eq!((scalar.display)(&sheet), "5");
        (scalar.write_str)(&mut sheet, "9").unwrap();
        assert_eq!((scalar.display)(&sheet), "9");
        let id = sheet.cell_names["empty"].0;
        sheet
            .write_compound::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>>(
                id,
                cel_runtime::Array::from_vec(vec![1, 2]),
            )
            .unwrap();
        assert_eq!((labels.cells[&id].display)(&sheet), "[1, 2]");
    }

    #[test]
    fn labels_from_cell_names_preserves_declaration_order() {
        let mut sheet = AdamSheet::new();
        let z = sheet.add_cell(1_i32);
        let a = sheet.add_cell(2_i32);

        let mut cell_names = IndexMap::new();
        cell_names.insert("z".to_string(), (z, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);
        let ids: Vec<_> = labels.cells.keys().copied().collect();
        assert_eq!(ids, vec![z, a]);
    }

    #[test]
    fn labels_from_cell_names_marks_numeric_cells_and_leaves_range_none_without_a_filter() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(3_i32);
        let b = sheet.add_cell(true);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("b".to_string(), (b, TypeShape::Named(TypeId::of::<bool>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert!(labels.cells[&a].is_numeric);
        assert!(labels.cells[&a].is_integer);
        assert!(labels.cells[&a].range.is_none());
        assert!(!labels.cells[&b].is_numeric);
    }

    #[test]
    fn labels_from_cell_names_populates_range_for_a_range_filtered_cell() {
        use adam_rs::Filter;
        use std::any::Any;

        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(50_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value, _args| Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>),
            |_args| {
                Some((
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                ))
            },
        );
        sheet.add_filter(a, filter).unwrap();

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        let range_fn = labels.cells[&a].range.as_ref().expect("range populated");
        assert_eq!(range_fn(&sheet), (0.0, 100.0));
    }

    fn sheet_with_one_cell() -> (AdamSheet, Labels) {
        let mut sheet = AdamSheet::new();
        let mut labels = Labels::new();
        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        (sheet, labels)
    }

    #[test]
    fn display_closure_returns_value_string() {
        let (sheet, labels) = sheet_with_one_cell();
        let a_id = *labels.cells.keys().next().unwrap();
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "2");
    }

    #[test]
    fn write_str_closure_parses_and_writes() {
        let (mut sheet, labels) = sheet_with_one_cell();
        let a_id = *labels.cells.keys().next().unwrap();
        assert!((labels.cells[&a_id].write_str)(&mut sheet, "5.0").is_ok());
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "5");
    }

    #[test]
    fn add_tuple_cell_display_returns_rust_debug_formatted_string() {
        let sheet = tuple_sheet();
        let cell_id = sheet.cell_names["pair"].0;
        let mut labels = Labels::new();
        labels.add_compound_cell(&sheet, cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        assert_eq!((meta.display)(&sheet), "(3, 4.5)");
    }

    #[test]
    fn add_tuple_cell_write_str_always_errs_without_mutating_the_sheet() {
        let mut sheet = tuple_sheet();
        let cell_id = sheet.cell_names["pair"].0;
        let mut labels = Labels::new();
        labels.add_compound_cell(&sheet, cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        let before = adam_lang::debug_compound_cell(&sheet, cell_id).unwrap();
        let result = (meta.write_str)(&mut sheet, "(1, 2.0)");
        assert!(result.is_err());
        let after = adam_lang::debug_compound_cell(&sheet, cell_id).unwrap();
        assert_eq!(before, after);
    }

    /// Builds real canonical tuple storage for host-display boundary contracts.
    fn tuple_sheet() -> adam_lang::ParsedSheet {
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
            .parse_str("sheet s { cell pair: (i32, f64) = (3, 4.5); }")
            .unwrap()
    }

    /// A missing compound formatting capability stays visible without enabling edits.
    #[test]
    fn compound_label_reports_unavailable_storage_without_panicking() {
        let mut sheet = AdamSheet::new();
        let id = sheet.add_cell(7i32);
        let mut labels = Labels::new();
        labels.add_compound_cell(&sheet, id, "unavailable");
        let meta = &labels.cells[&id];
        assert_eq!((meta.display)(&sheet), "?");
        assert!((meta.write_str)(&mut sheet, "9").is_err());
        assert_eq!(*sheet.read::<i32>(id).unwrap(), 7);
    }
}
