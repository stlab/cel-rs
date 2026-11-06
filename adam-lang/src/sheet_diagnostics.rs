//! Owned source and provenance metadata for errors that outlive a sheet.
//!
//! Capture diagnostics before a sheet mutation that can fail. The snapshot keeps
//! source owners and native error-site provenance after the sheet is discarded.
//!
//! # Examples
//!
//! ```
//! use adam_lang::{AdamParser, TypeRegistry};
//! let mut parsed = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
//!     .parse_str("sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }")?;
//! let diagnostics = parsed.diagnostics();
//! let error = parsed.propagate().unwrap_err();
//! drop(parsed);
//! assert!(!diagnostics.locate_error(&error).is_empty());
//! # Ok::<(), cel_parser::ParseError>(())
//! ```

use std::collections::HashMap;

use adam_rs::{CellId, RelationshipId, RequirementId};
use indexmap::IndexMap;

use crate::{
    ParsedSheet, SourceId, SourceLabel, composition::CellOrigin, composition::InstanceOrigin,
    type_registry::TypeShape,
};

/// Retains source and native provenance without owning or borrowing a `Sheet`.
///
/// The snapshot owns names, spans, source identities, requirement labels, and
/// expansion provenance. Its immutable source owners and authored records may
/// share backing storage with the parsed model.
///
/// # Examples
///
/// ```
/// let diagnostics = adam_lang::ParsedSheet::new().diagnostics();
/// assert!(diagnostics.sources().is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct SheetDiagnostics {
    /// Public root cell names and their IDs and complete shapes, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Public output names and their IDs, in declaration order.
    pub output_names: IndexMap<String, CellId>,
    /// Explicit requirement names keyed by runtime requirement ID.
    pub requirement_names: HashMap<RequirementId, String>,
    /// Requirement IDs mapped to authored source labels, including unnamed checks.
    pub requirement_labels: HashMap<RequirementId, SourceLabel>,
    /// Root-local method spans keyed by relationship ID and method index.
    pub method_spans: HashMap<(RelationshipId, usize), cel_parser::SourceSpan>,
    /// Root-local relationship spans keyed by relationship ID.
    pub relationship_spans: HashMap<RelationshipId, cel_parser::SourceSpan>,
    /// Root-local declaration spans keyed by cell ID.
    pub cell_spans: HashMap<CellId, cel_parser::SourceSpan>,
    /// Retained source owners in insertion order.
    pub sources: Vec<SourceId>,
    /// Anonymous convenience source used for caller-selected display names.
    pub anonymous_source: Option<SourceId>,
    /// Parent-linked native occurrence provenance.
    pub instance_origins: Vec<InstanceOrigin>,
    /// Authored method origins keyed by runtime relationship and method index.
    pub method_origins: HashMap<(RelationshipId, usize), adam_rs::MethodOrigin>,
    /// Authored cell origins keyed by runtime cell ID.
    pub cell_origins: HashMap<CellId, Vec<CellOrigin>>,
    /// Complete shapes for root and private cells.
    pub cell_shapes: HashMap<CellId, TypeShape>,
}

impl SheetDiagnostics {
    /// Creates an owned metadata snapshot from a parsed sheet.
    ///
    /// - Complexity: O(m) time and space for retained metadata size m; immutable
    ///   source owners and authored item slices are shared.
    pub(crate) fn from_parsed(parsed: &ParsedSheet) -> Self {
        Self {
            cell_names: parsed.cell_names.clone(),
            output_names: parsed.output_names.clone(),
            requirement_names: parsed.requirement_names.clone(),
            requirement_labels: parsed.requirement_labels.clone(),
            method_spans: parsed.method_spans.clone(),
            relationship_spans: parsed.relationship_spans.clone(),
            cell_spans: parsed.cell_spans.clone(),
            sources: parsed.sources.clone(),
            anonymous_source: parsed.anonymous_source.clone(),
            instance_origins: parsed.instance_origins.clone(),
            method_origins: parsed.method_origins.clone(),
            cell_origins: parsed.cell_origins.clone(),
            cell_shapes: parsed.cell_shapes.clone(),
        }
    }

    /// Resolves an error to owning definition and invocation labels.
    ///
    /// - Complexity: O(o + m + c + k + p × a + q × a + l²) time and
    ///   O(o + m + c + k + l) space for occurrences o, methods m, public cells c,
    ///   conditionals k, candidate cell origins p, visited occurrence/ancestor records q,
    ///   local authored items a, and returned labels l.
    ///
    /// # Examples
    ///
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// let mut parsed = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }")?;
    /// let diagnostics = parsed.diagnostics();
    /// let error = parsed.propagate().unwrap_err();
    /// assert!(!diagnostics.locate_error(&error).is_empty());
    /// # Ok::<(), cel_parser::ParseError>(())
    /// ```
    pub fn locate_error(&self, error: &adam_rs::Error) -> Vec<SourceLabel> {
        crate::error_labels::locate_error(self, error)
    }

    /// Borrows retained source owners in their original insertion order.
    ///
    /// # Examples
    ///
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// let parsed = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { cell x: i32 = 1; }")?;
    /// assert_eq!(parsed.diagnostics().sources().len(), 1);
    /// # Ok::<(), cel_parser::ParseError>(())
    /// ```
    pub fn sources(&self) -> &[SourceId] {
        &self.sources
    }

    /// Borrows the anonymous convenience source used for caller-selected display names.
    ///
    /// # Examples
    ///
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// let parsed = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { cell x: i32 = 1; }")?;
    /// assert_eq!(parsed.diagnostics().anonymous_source().unwrap().file_name(), "<input>");
    /// # Ok::<(), cel_parser::ParseError>(())
    /// ```
    pub fn anonymous_source(&self) -> Option<&SourceId> {
        self.anonymous_source.as_ref()
    }
}

/// Supplies the metadata needed to resolve native runtime error sites.
pub(crate) trait ErrorMetadata {
    /// Returns public cell names in declaration order.
    fn cell_names(&self) -> &IndexMap<String, (CellId, TypeShape)>;

    /// Returns retained occurrence provenance.
    fn instance_origins(&self) -> &[InstanceOrigin];

    /// Returns retained method provenance.
    fn method_origins(&self) -> &HashMap<(RelationshipId, usize), adam_rs::MethodOrigin>;

    /// Returns retained cell provenance.
    fn cell_origins(&self) -> &HashMap<CellId, Vec<CellOrigin>>;
}

impl ErrorMetadata for ParsedSheet {
    /// Returns public cell names in declaration order.
    fn cell_names(&self) -> &IndexMap<String, (CellId, TypeShape)> {
        &self.cell_names
    }

    /// Returns retained occurrence provenance.
    fn instance_origins(&self) -> &[InstanceOrigin] {
        &self.instance_origins
    }

    /// Returns retained method provenance.
    fn method_origins(&self) -> &HashMap<(RelationshipId, usize), adam_rs::MethodOrigin> {
        &self.method_origins
    }

    /// Returns retained cell provenance.
    fn cell_origins(&self) -> &HashMap<CellId, Vec<CellOrigin>> {
        &self.cell_origins
    }
}

impl ErrorMetadata for SheetDiagnostics {
    /// Returns public cell names in declaration order.
    fn cell_names(&self) -> &IndexMap<String, (CellId, TypeShape)> {
        &self.cell_names
    }

    /// Returns retained occurrence provenance.
    fn instance_origins(&self) -> &[InstanceOrigin] {
        &self.instance_origins
    }

    /// Returns retained method provenance.
    fn method_origins(&self) -> &HashMap<(RelationshipId, usize), adam_rs::MethodOrigin> {
        &self.method_origins
    }

    /// Returns retained cell provenance.
    fn cell_origins(&self) -> &HashMap<CellId, Vec<CellOrigin>> {
        &self.cell_origins
    }
}
