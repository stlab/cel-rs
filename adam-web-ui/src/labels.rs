//! Cell display/write metadata and diagnostic formatting for a live [`adam_rs::Sheet`].
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] keys, driving [`crate::SheetInspector`]'s rendering.
//! [`format_adam_error`] formats an [`adam_rs::Error`] as a rustc-style diagnostic when
//! possible.

use adam_lang::type_registry::TypeShape;
use adam_rs::{CellId, Error, Sheet};
use annotate_snippets::Renderer;
use cel_parser::FormatRustcStyle;
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
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses `s` and writes the result to the cell; returns `Err` on parse failure or type
    /// mismatch. May also always return `Err` for a cell type with no write support yet (e.g.
    /// tuples — see [`Labels::add_tuple_cell`]).
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
                display: Box::new(move |sheet| {
                    sheet
                        .read::<T>(id)
                        .map(|v| format!("{}", v))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(move |sheet, s| {
                    let value = s
                        .parse::<T>()
                        .map_err(|e| Error::MethodFailed(anyhow::anyhow!("parse error: {}", e)))?;
                    sheet.write(id, value)
                }),
                range: None,
            },
        );
    }

    /// Registers display-only metadata for a tuple-typed cell of any shape.
    ///
    /// `write_str` always returns `Err` — no tuple-literal parser exists yet (tracked as a
    /// follow-up: see the "Support editing tuple-typed cells in `begin`" GitHub issue). The
    /// field still participates fully in [`crate::SheetInspector`]'s existing
    /// invalid/warning/disabled machinery, since that's entirely keyed on `CellId`, not on any
    /// per-type behavior.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with, holding
    ///   a `cel_runtime::DynamicSequence`.
    pub fn add_tuple_cell(&mut self, id: CellId, label: &str) {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                is_bool: false,
                is_numeric: false,
                display: Box::new(move |sheet| {
                    sheet
                        .read::<cel_runtime::DynamicSequence>(id)
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed(anyhow::anyhow!(
                        "editing tuple-typed cells is not yet supported"
                    )))
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
/// `adam_lang::TypeRegistry::new()` registers. A tuple-typed cell
/// (`TypeShape::Tuple`) appears with a Debug-formatted, display-only entry via
/// [`Labels::add_tuple_cell`]. Cells whose `TypeId` is none of the built-in
/// primitives are silently skipped, so they simply won't appear in the sidebar.
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
            TypeShape::Tuple(_) => {
                labels.add_tuple_cell(id, name);
                continue;
            }
        };
        macro_rules! try_numeric_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    mark_numeric::<$T>(&mut labels, sheet, id);
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
                    mark_numeric::<$T>(&mut labels, sheet, id);
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

/// Marks `id`'s `CellMeta` as numeric and, if `sheet.filter_kind(id)` is a range clamp,
/// populates its live-range closure.
fn mark_numeric<T: std::any::Any + Clone + ToF64Display>(
    labels: &mut Labels,
    sheet: &Sheet,
    id: CellId,
) {
    let Some(meta) = labels.cells.get_mut(&id) else {
        return;
    };
    meta.is_numeric = true;
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
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source`, ANSI-colored for a
/// terminal, with `file_name` (e.g. `"begin/examples/toy_example.adm2"`) shown
/// in the diagnostic header. All other variants have no source span and fall
/// back to their `Display` message, ignoring `file_name`.
pub fn format_adam_error(e: &Error, source: &str, file_name: &str) -> String {
    match e {
        Error::MethodFailed(inner) => {
            inner.format_rustc_style(source, file_name, 1, &Renderer::styled())
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_rs::Sheet as AdamSheet;

    #[test]
    fn format_adam_error_invalid_id_falls_back_to_display() {
        let msg = format_adam_error(&Error::InvalidId, "source text", "test.adm2");
        assert_eq!(msg, "invalid cell or relationship id");
    }

    #[test]
    fn format_adam_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed(inner);

        let msg = format_adam_error(&err, source, "test.adm2");

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
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
        let mut sheet = AdamSheet::new();
        let pair = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));

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
        let mut sheet = AdamSheet::new();
        let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
        let mut labels = Labels::new();
        labels.add_tuple_cell(cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        assert_eq!((meta.display)(&sheet), "(3, 4.5)");
    }

    #[test]
    fn add_tuple_cell_write_str_always_errs_without_mutating_the_sheet() {
        let mut sheet = AdamSheet::new();
        let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
        let mut labels = Labels::new();
        labels.add_tuple_cell(cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        let before = sheet
            .read::<cel_runtime::DynamicSequence>(cell_id)
            .unwrap()
            .clone();
        let result = (meta.write_str)(&mut sheet, "(1, 2.0)");
        assert!(result.is_err());
        let after = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        assert_eq!(&before, after);
    }
}
