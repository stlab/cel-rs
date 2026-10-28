//! Builds live roots through the checked module compiler and native instantiator.
//!
//! See the crate root's [`# Grammar`](crate#grammar) section for the full EBNF.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::str::FromStr;

use indexmap::IndexMap;

use adam_rs::{CellId, ErrorSite, RelationshipId, RequirementId, Sheet};
use cel_parser::lex_lexer::{HasSpan, LexLexer};
use cel_parser::{OpLookup, ParseError, SourceSpan};
use proc_macro2::{Span, TokenStream};

use crate::TypeRegistry;
use crate::cell_value::{CellValue, cell_type_id};
use crate::type_registry::TypeShape;

/// Parser result type.
pub type Result<T> = std::result::Result<T, ParseError>;

/// Checks a complete shape against an existing canonical descriptor without a registry.
///
/// - Complexity: O(n) descriptor nodes.
fn shape_matches(shape: &TypeShape, value: &cel_runtime::ValueType) -> bool {
    use cel_runtime::ValueKind;
    match (shape, value.kind()) {
        (TypeShape::Named(a), ValueKind::Leaf(b)) => a == b,
        (TypeShape::Array(a), ValueKind::Array(b)) => shape_matches(a, b),
        (TypeShape::Tuple(a), ValueKind::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| shape_matches(a, &b.value_type))
        }
        (
            TypeShape::Callable(a, ar),
            ValueKind::Callable {
                parameters: b,
                result: br,
            },
        ) => {
            a.len() == b.len()
                && a.iter().zip(b).all(|(a, b)| shape_matches(a, b))
                && shape_matches(ar, br)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// ParsedSheet
// ---------------------------------------------------------------------------

/// Owns a live [`Sheet`], ordered public root names, complete shapes and provenance.
///
/// Derefs to [`Sheet`] so callers that only need sheet methods (e.g.
/// `propagate`) can use the result exactly as if it were a `Sheet`.
/// [`Self::new`] constructs a source-free destination; [`AdamParser::parse_str`]
/// retains the ordinary source-parsing convenience.
///
/// # Examples
///
/// ```
/// use adam_lang::{AdamParser, TypeRegistry};
/// use cel_parser::OpLookup;
/// let parsed = AdamParser::new(TypeRegistry::new(), OpLookup::new())
///     .parse_str("sheet s { cell x: i32 = 1 require { @positive x > 0i32; }; }")?;
/// let id = parsed.cell_requirements(parsed.cell_names["x"].0).unwrap()[0];
/// assert_eq!(parsed.requirement_names[&id], "positive");
/// # Ok::<(), cel_parser::ParseError>(())
/// ```
pub struct ParsedSheet {
    /// The constructed sheet.
    pub sheet: Sheet,
    /// Cell name → `(CellId, TypeShape)`, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Output name → `CellId`, in declaration order — parity with `cell_names`, for callers
    /// that need to look up `Sheet::cell_requirements_valid`/`Sheet::violated_requirements` by
    /// name.
    pub output_names: IndexMap<String, CellId>,
    /// Explicit `@name` labels keyed by requirement ID; unnamed requirements have no entry.
    /// Lookup takes expected O(1) time. Iterate the sheet's requirement IDs, not this map,
    /// to preserve attachment or violation order.
    pub requirement_names: HashMap<RequirementId, String>,
    /// `(RelationshipId, method index)` → the source span of a root-local binding.
    /// Lets a caller translate an `adam_rs::Error`'s
    /// `ErrorSite::Method` (raised well after parsing, e.g. from `Sheet::propagate`) back
    /// to a source location.
    ///
    /// This legacy single-source view excludes included component methods and output
    /// writers. Their complete owning metadata remains in `method_origins` and
    /// `instance_origins`.
    pub method_spans: HashMap<(RelationshipId, usize), SourceSpan>,
    /// `RelationshipId` → the source span of a root-local `relationship { ... }` block.
    /// Resolves an `adam_rs::Error`'s
    /// `ErrorSite::Relationship` (e.g. from `Error::Conflict`) back to a source location.
    pub relationship_spans: HashMap<RelationshipId, SourceSpan>,
    /// `CellId` → the source span of a root-local `cell`, `source`, or `out` name.
    /// Included private declarations retain their owning locations in `cell_origins`.
    pub cell_spans: HashMap<CellId, SourceSpan>,
    /// Retains independent source identities used by instantiated definitions.
    pub sources: Vec<crate::SourceId>,
    /// Identifies the anonymous convenience frontend's source for display-name overrides.
    pub(crate) anonymous_source: Option<crate::SourceId>,
    /// Retains parent-linked provenance for direct template invocations.
    pub instance_origins: Vec<crate::composition::InstanceOrigin>,
    /// Maps compact live method indices to occurrence and authored method identity.
    pub method_origins: HashMap<(RelationshipId, usize), adam_rs::MethodOrigin>,
    /// Retains every occurrence-local declaration/formal mapped to each live cell.
    pub cell_origins: HashMap<CellId, Vec<crate::composition::CellOrigin>>,
    /// Retains full private shapes without exposing private names in root maps.
    pub(crate) cell_shapes: HashMap<CellId, TypeShape>,
}

impl std::fmt::Debug for ParsedSheet {
    /// Formats public name and root-span metadata without evaluating the sheet.
    ///
    /// - Complexity: O(b) time and formatter output space for b rendered bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedSheet")
            .field("cell_names", &self.cell_names)
            .field("output_names", &self.output_names)
            .field("requirement_names", &self.requirement_names)
            .field("method_spans", &self.method_spans)
            .field("relationship_spans", &self.relationship_spans)
            .field("cell_spans", &self.cell_spans)
            .finish()
    }
}

impl Default for ParsedSheet {
    /// Constructs an empty source-free destination.
    fn default() -> Self {
        Self::new()
    }
}

impl ParsedSheet {
    /// Retains the legacy root-only span views from owner-checked native identities.
    ///
    /// Private occurrences remain in the owning provenance tables, never in these
    /// unqualified single-source maps.
    /// - Precondition: `instance` is this destination's most recent root expansion.
    /// - Complexity: O(a + m) authored root records and live method origins.
    pub(crate) fn retain_root_spans(&mut self, instance: &adam_rs::Instance) {
        let occurrence = &self
            .instance_origins
            .last()
            .expect("root expansion retained")
            .occurrences[0];
        debug_assert_eq!(&occurrence.id, instance.occurrences()[0].id());
        let mut methods = HashMap::new();
        for item in occurrence.authored_items() {
            let Some(location) = &item.label.location else {
                continue;
            };
            if let Some(slot) = &item.cell
                && matches!(
                    item.role,
                    crate::AuthoredRole::Cell
                        | crate::AuthoredRole::Source
                        | crate::AuthoredRole::Out
                )
            {
                let id = instance
                    .cell(&occurrence.id, slot)
                    .expect("authored local slot");
                self.cell_spans.insert(id, location.span);
            }
            if matches!(
                item.role,
                crate::AuthoredRole::Relationship | crate::AuthoredRole::Method
            ) && let Some(slot) = &item.relationship
            {
                for &relationship in instance
                    .relationship_group(&occurrence.id, slot)
                    .expect("authored root relationship slot")
                {
                    if item.role == crate::AuthoredRole::Relationship {
                        self.relationship_spans.insert(relationship, location.span);
                    }
                    if let Some(index) = item.method_index {
                        methods.insert((relationship, index), location.span);
                    }
                }
            }
        }
        for (relationship, live, origin) in instance.method_origins() {
            if origin.occurrence != occurrence.id {
                continue;
            }
            if let Some(span) = methods.get(&(relationship, origin.authored_index)) {
                self.method_spans.insert((relationship, live), *span);
            }
        }
    }

    /// Retains native validation causes and source-owning site labels, primary cell first.
    ///
    /// Falls back to the real root declaration only when no live site resolves.
    /// - Precondition: `instance` identifies this destination's root expansion.
    /// - Complexity: O(c + a + s) public names, expanded authored records and native sites.
    pub(crate) fn validation_diagnostic(
        &self,
        error: adam_rs::Error,
        root: &crate::SourceLabel,
        instance: &adam_rs::Instance,
    ) -> crate::ModuleDiagnostic {
        let mut labels = Vec::new();
        let wanted: HashSet<_> = error
            .sites()
            .iter()
            .filter_map(|site| {
                if let ErrorSite::Relationship(id) = site {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        let mut relationships = HashMap::new();
        for occurrence in &self
            .instance_origins
            .last()
            .expect("root expansion retained")
            .occurrences
        {
            for item in occurrence.authored_items() {
                if matches!(
                    item.role,
                    crate::AuthoredRole::Relationship | crate::AuthoredRole::Out
                ) && let Some(slot) = &item.relationship
                {
                    for &id in instance
                        .relationship_group(&occurrence.id, slot)
                        .expect("authored relationship slot")
                    {
                        if wanted.contains(&id) {
                            relationships.insert(id, &item.label);
                        }
                    }
                }
            }
        }
        let names: HashMap<_, _> = self
            .cell_names
            .iter()
            .map(|(name, (id, _))| (*id, name))
            .collect();
        let name = |id| names.get(&id).map(|name| (*name).clone());
        for (index, site) in error.sites().iter().enumerate() {
            let label = match site {
                ErrorSite::Cell(id) => self
                    .cell_origins
                    .get(id)
                    .and_then(|origins| origins.first())
                    .map(|origin| &origin.label),
                ErrorSite::Relationship(id) => relationships.get(id).copied(),
                _ => None,
            };
            if let Some(label) = label {
                let mut label = label.clone();
                label.label = crate::error_labels::site_label(&error, index, &name);
                labels.push(label);
            }
        }
        if labels.is_empty() {
            labels.push(root.clone());
        }
        crate::ModuleDiagnostic {
            message: error.to_string(),
            labels,
            cause: Some(crate::ModuleCause::new(error)),
        }
    }

    /// Constructs an empty source-free destination without invoking a parser.
    ///
    /// Register manually added root cells with [`Self::register_cell`] before binding.
    /// # Examples
    /// ```
    /// assert_eq!(adam_lang::ParsedSheet::new().cells().count(), 0);
    /// ```
    pub fn new() -> Self {
        Self {
            sheet: Sheet::new(),
            cell_names: IndexMap::new(),
            output_names: IndexMap::new(),
            requirement_names: HashMap::new(),
            method_spans: HashMap::new(),
            relationship_spans: HashMap::new(),
            cell_spans: HashMap::new(),
            sources: Vec::new(),
            anonymous_source: None,
            instance_origins: Vec::new(),
            method_origins: HashMap::new(),
            cell_origins: HashMap::new(),
            cell_shapes: HashMap::new(),
        }
    }

    /// Registers an existing root cell's checked shape and public name.
    ///
    /// Rejection leaves metadata and graph unchanged. Private cells are registered
    /// automatically without root names by typed template instantiation.
    /// Root output cells also enter `output_names`.
    /// # Errors
    /// Rejects duplicate names, unknown shapes/IDs, storage or complete-shape mismatches.
    /// - Complexity: Expected O(n + l) shape nodes and name bytes.
    ///
    /// # Examples
    /// See [`crate::composition`].
    pub fn register_cell(
        &mut self,
        name: &str,
        id: CellId,
        shape: TypeShape,
        types: &TypeRegistry,
    ) -> std::result::Result<(), crate::ModuleDiagnostic> {
        types
            .validate_shape(&shape)
            .map_err(crate::composition::diagnostic)?;
        if self.cell_names.contains_key(name) {
            return Err(crate::composition::diagnostic("duplicate root cell name"));
        }
        self.validate_cell_shape(id, &shape)
            .map_err(crate::composition::diagnostic)?;
        if self.sheet.cell_kind(id) == Some(adam_rs::CellKind::Out) {
            self.output_names.insert(name.into(), id);
        }
        self.cell_shapes.insert(id, shape.clone());
        self.cell_names.insert(name.into(), (id, shape));
        Ok(())
    }

    /// Adds a source-free ordinary compound cell with a checked canonical witness.
    ///
    /// # Errors
    /// Rejects duplicate names, unknown leaves, or missing cell clone/equality capabilities
    /// before graph mutation.
    /// - Complexity: O(n) shape nodes plus value capability preparation.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{Array, ArrayOf, Leaf};
    /// let mut sheet = adam_lang::ParsedSheet::new();
    /// let id = sheet.add_compound_cell::<ArrayOf<Leaf<i32>>>(
    ///     "xs", Array::from_vec(vec![1]), &adam_lang::TypeRegistry::new())?;
    /// assert_eq!(sheet.read_compound::<ArrayOf<Leaf<i32>>>(id)?.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn add_compound_cell<R: cel_runtime::ValueRepr>(
        &mut self,
        name: &str,
        value: R::Value,
        types: &TypeRegistry,
    ) -> std::result::Result<CellId, crate::ModuleDiagnostic> {
        let shape = types
            .shape_of(&R::value_type())
            .map_err(crate::composition::diagnostic)?;
        if matches!(shape, TypeShape::Named(_)) || self.cell_names.contains_key(name) {
            return Err(crate::composition::diagnostic(
                "expected compound shape and unused root name",
            ));
        }
        let descriptor = types
            .checked_value_type(&shape)
            .map_err(crate::composition::diagnostic)?;
        CellValue::validate(&descriptor).map_err(crate::composition::diagnostic)?;
        let owned = cel_runtime::OwnedValue::new::<R>(value)
            .with_value_type(descriptor)
            .map_err(crate::composition::diagnostic)?;
        let id = self.sheet.add_cell(CellValue(owned));
        self.cell_shapes.insert(id, shape.clone());
        self.cell_names.insert(name.into(), (id, shape));
        Ok(id)
    }

    /// Borrows a root or hygienically private cell's complete registered shape.
    ///
    /// Returns `None` for unregistered handles.
    ///
    /// # Examples
    /// ```
    /// assert!(adam_lang::ParsedSheet::new().cell_names.is_empty());
    /// ```
    pub fn cell_shape(&self, id: CellId) -> Option<&TypeShape> {
        self.cell_shapes.get(&id)
    }

    /// Checks a live cell's concrete storage and full compound identity.
    ///
    /// # Errors
    /// Rejects unknown handles, different storage or recursive shape.
    /// - Complexity: O(n) compound shape nodes.
    pub(crate) fn validate_cell_shape(&self, id: CellId, shape: &TypeShape) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.sheet.cell_type_id(id) == Some(cell_type_id(shape)),
            "invalid actual storage"
        );
        if !matches!(shape, TypeShape::Named(_)) {
            let actual = self.sheet.read::<CellValue>(id)?;
            // Compare structural identity directly; no registry or carrier inference is needed.
            anyhow::ensure!(
                shape_matches(shape, actual.0.value_type()),
                "complete shape mismatch"
            );
        }
        Ok(())
    }

    /// Clones a compound cell into its exact canonical witness representation.
    ///
    /// # Errors
    /// Rejects unknown cells, concrete leaf storage, and recursive shape mismatches.
    /// - Complexity: O(n + v) validation and cloning for n shape nodes and v values.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_runtime::{ArrayOf, Leaf};
    /// let sheet = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { cell xs: [i32]; }").unwrap();
    /// assert!(sheet.read_compound::<ArrayOf<Leaf<i32>>>(sheet.cell_names["xs"].0)?.is_empty());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn read_compound<R: cel_runtime::ValueRepr>(&self, id: CellId) -> anyhow::Result<R::Value> {
        let cell = self.sheet.read::<CellValue>(id)?;
        anyhow::ensure!(
            cell.0.value_type().same_shape(&R::value_type()),
            "compound cell representation mismatch"
        );
        // SAFETY: The checked host boundary just validated the complete witness.
        Ok(unsafe { cell.0.try_clone()?.into_typed_bound::<R>() })
    }

    /// Writes a compound aspiration after validating its complete witness identity.
    ///
    /// Existing cell capabilities, rather than witness defaults, govern propagation.
    /// # Errors
    /// Rejects unknown cells, leaf storage, or any recursive shape mismatch before writing.
    /// - Complexity: O(n) validation and descriptor construction for n shape nodes.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_runtime::{Array, ArrayOf, Leaf};
    /// let mut sheet = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { source xs: [i32]; }").unwrap();
    /// sheet.write_compound::<ArrayOf<Leaf<i32>>>(sheet.cell_names["xs"].0, Array::from_vec(vec![1]))?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn write_compound<R: cel_runtime::ValueRepr>(
        &mut self,
        id: CellId,
        value: R::Value,
    ) -> anyhow::Result<()> {
        let descriptor = self.sheet.read::<CellValue>(id)?.0.value_type().clone();
        let value = cel_runtime::OwnedValue::new::<R>(value).with_value_type(descriptor)?;
        self.sheet.write(id, CellValue(value))?;
        Ok(())
    }

    /// Clones the original compound aspiration, independently of any filter result.
    ///
    /// # Errors
    /// Rejects unknown cells, concrete leaf storage, or a mismatched recursive witness.
    /// - Complexity: O(n + v) for n descriptor nodes and v cloned values.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_runtime::{ArrayOf, Leaf};
    /// let sheet = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { source xs: [i32]; }").unwrap();
    /// assert!(sheet.source_compound::<ArrayOf<Leaf<i32>>>(sheet.cell_names["xs"].0)?.is_empty());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn source_compound<R: cel_runtime::ValueRepr>(
        &self,
        id: CellId,
    ) -> anyhow::Result<R::Value> {
        let cell = self.sheet.source::<CellValue>(id)?;
        anyhow::ensure!(
            cell.0.value_type().same_shape(&R::value_type()),
            "compound cell representation mismatch"
        );
        // SAFETY: The complete witness was just checked at this host boundary.
        Ok(unsafe { cell.0.try_clone()?.into_typed_bound::<R>() })
    }

    /// Resolves native error sites to owning labels and actual invocation context.
    ///
    /// Definition labels precede root-to-child invocation context. Inner CEL spans
    /// resolve only against the failed method's definition owner. Native definitions
    /// retain labels without locations; unknown sites are skipped.
    ///
    /// - Complexity: O(o + m + c + q × a + l²) time and O(o + m + c + l)
    ///   space for occurrences o, methods m, public cells c, visited occurrence/ancestor
    ///   records q, local authored items a and returned labels l.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_parser::OpLookup;
    ///
    /// let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    /// // Two relationships forming an algebraic loop with no external source for either
    /// // cell: `x` needs `y` and `y` needs `x`, so `propagate` can't pick a valid method
    /// // assignment and returns `Error::Cycle`.
    /// let mut parsed = parser
    ///     .parse_str(
    ///         "sheet s { cell x: i32 = 0; cell y: i32 = 0; \
    ///          relationship { x := y + 1i32; } relationship { y := x + 1i32; } }",
    ///     )
    ///     .unwrap();
    /// let e = parsed.propagate().unwrap_err();
    /// let located = parsed.locate_error(&e);
    /// assert!(!located.is_empty());
    /// assert!(!located[0].label.is_empty());
    /// ```
    pub fn locate_error(&self, e: &adam_rs::Error) -> Vec<crate::SourceLabel> {
        crate::error_labels::locate_error(self, e)
    }

    /// Borrows the anonymous frontend source whose display name the caller supplies.
    ///
    /// Explicit module roots and programmatic sheets return `None`; their owning
    /// file names must not be replaced by a renderer's caller-source arguments.
    ///
    /// # Examples
    /// ```
    /// assert!(adam_lang::ParsedSheet::new().anonymous_source().is_none());
    /// ```
    pub fn anonymous_source(&self) -> Option<&crate::SourceId> {
        self.anonymous_source.as_ref()
    }
}

impl std::ops::Deref for ParsedSheet {
    /// Exposes the underlying runtime constraint sheet.
    type Target = Sheet;

    /// Borrows the underlying runtime sheet.
    fn deref(&self) -> &Sheet {
        &self.sheet
    }
}

impl std::ops::DerefMut for ParsedSheet {
    /// Exclusively borrows the underlying runtime sheet.
    fn deref_mut(&mut self) -> &mut Sheet {
        &mut self.sheet
    }
}

/// Owns one immutable type/operation environment for repeatable root construction.
///
/// Install libraries and register types before construction. Each parse creates a
/// fresh module collection and live graph; no execution state is shared.
///
/// # Examples
/// ```
/// let mut parser = adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(),
///     cel_parser::OpLookup::new());
/// let parsed = parser.parse_str("sheet app { cell x: i32 = 7; }")?;
/// assert_eq!(*parsed.read::<i32>(parsed.cell_names["x"].0).unwrap(), 7);
/// # Ok::<(), cel_parser::ParseError>(())
/// ```
pub struct AdamParser {
    environment: Rc<crate::module::compile::Environment>,
}

impl AdamParser {
    /// Takes ownership of types and operations, shared immutably with compiled factories.
    ///
    /// # Examples
    /// See [`AdamParser`].
    pub fn new(types: TypeRegistry, lookup: OpLookup) -> Self {
        Self {
            environment: Rc::new(crate::module::compile::Environment {
                types: Rc::new(types),
                lookup,
            }),
        }
    }

    /// Constructs the sole root in a source module without propagation.
    ///
    /// Reusable definitions may accompany the root. All definitions and inactive
    /// branches check before construction. Zero or multiple roots require explicit
    /// selection through [`crate::CompiledModules::instantiate_root`].
    ///
    /// # Errors
    /// Returns syntax, checking, selection, construction or combined validation
    /// errors. The error source retains the original owning [`crate::ModuleDiagnostic`].
    /// - Complexity: Module compilation, native construction and combined validation;
    ///   adapting a diagnostic additionally scans O(n) source tokens.
    ///
    /// # Examples
    /// See [`AdamParser`].
    pub fn parse_str(&mut self, source: &str) -> Result<ParsedSheet> {
        let path: crate::ModulePath = "input".parse().expect("fixed valid module path");
        let mut modules = crate::ModuleSet::new();
        modules
            .insert_source(path.clone(), "<input>", source)
            .map_err(|error| frontend_error(source, error))?;
        let compiled = crate::module::compile::compile(self.environment.clone(), modules)
            .map_err(|mut errors| frontend_error(source, errors.remove(0)))?;
        let mut roots = compiled.roots(&path);
        let name = roots.next().ok_or_else(|| {
            frontend_error(
                source,
                crate::composition::diagnostic("expected one root sheet"),
            )
        })?;
        if roots.next().is_some() {
            let labels = compiled
                .roots(&path)
                .filter_map(|name| {
                    compiled
                        .prepared_root(&path, name)
                        .map(|root| root.definition_label().clone())
                })
                .collect();
            return Err(frontend_error(
                source,
                crate::ModuleDiagnostic {
                    message: "ambiguous roots: expected one root sheet".into(),
                    labels,
                    cause: None,
                },
            ));
        }
        let mut parsed = compiled
            .instantiate_root(&path, name)
            .map_err(|error| frontend_error(source, error))?;
        parsed.anonymous_source = Some(
            compiled
                .sources()
                .first()
                .expect("the convenience frontend retains its supplied source")
                .clone(),
        );
        Ok(parsed)
    }
}

/// Adapts a module error to the existing single-source parser boundary without losing it.
///
/// Real token positions supply proc-macro spans; source-owning labels and concrete
/// native causes remain available through `Error::source`.
/// - Complexity: O(n + l) source tokens and diagnostic labels.
fn frontend_error(source: &str, error: crate::ModuleDiagnostic) -> ParseError {
    let location = error
        .labels
        .iter()
        .find_map(|label| label.location.as_ref());
    let mut first = Span::call_site();
    let mut last = None;
    match TokenStream::from_str(source) {
        Ok(tokens) => {
            if let Some(location) = location {
                for token in LexLexer::new(tokens.into_iter()) {
                    let span = token.span();
                    if span.start() == location.span.start {
                        first = span;
                    }
                    if span.end() == location.span.end {
                        last = Some(span);
                    }
                }
            }
        }
        Err(lexical) => {
            let lexical = ParseError::from_lex_error(source, lexical);
            first = lexical.span();
            last = lexical.end_span();
        }
    }
    let mut adapted = if let Some(last) = last {
        ParseError::new_range(error.message.clone(), first, last)
    } else {
        ParseError::new(error.message.clone(), first)
    };
    adapted = adapted.with_secondary(
        error
            .labels
            .iter()
            .skip(1)
            .filter_map(|label| {
                label
                    .location
                    .as_ref()
                    .map(|location| cel_parser::SpanLabel {
                        span: location.span,
                        label: label.label.clone(),
                    })
            })
            .collect(),
    );
    adapted.with_cause(error)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TypeRegistry;
    use cel_parser::OpLookup;
    use cel_runtime::{ArrayOf, Leaf};
    use std::any::Any;

    /// Creates an executable parser with built-in types and operations.
    fn parser() -> AdamParser {
        AdamParser::new(TypeRegistry::new(), OpLookup::new())
    }

    /// Identifies a registered test leaf distinct from built-in integers.
    #[derive(Clone, Debug, PartialEq)]
    struct Custom(i32);

    /// Identifies the custom array-counting callable in test lookup.
    #[derive(Clone)]
    struct CountFn;

    /// Resolves custom test leaves and array-counting calls.
    fn custom_lookup() -> OpLookup {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _span| match name {
            "left" if arity == 0 => {
                segment.op0(|| Custom(1));
                Ok(true)
            }
            "right" if arity == 0 => {
                segment.op0(|| Custom(2));
                Ok(true)
            }
            "count" if arity == 0 => {
                segment.op0(|| CountFn);
                Ok(true)
            }
            "()" if arity == 2 => {
                if segment.peek_stack_infos(1)[0]
                    .value_type
                    .array_element()
                    .is_some_and(|element| element.array_element().is_some())
                {
                    segment
                        .op2_repr::<Leaf<CountFn>, ArrayOf<ArrayOf<Leaf<Custom>>>, Leaf<i32>, _>(
                            |_callee, array| array.len() as i32,
                        )?;
                } else {
                    segment.op2_repr::<Leaf<CountFn>, ArrayOf<Leaf<Custom>>, Leaf<i32>, _>(
                        |_callee, array| array.len() as i32,
                    )?;
                }
                Ok(true)
            }
            _ => Ok(false),
        });
        lookup
    }

    /// Accepts an empty executable sheet.
    #[test]
    fn parse_empty_sheet() {
        let _sheet = parser().parse_str("sheet empty {}").unwrap();
    }

    /// Accepts a cell with a matching annotation and initializer.
    #[test]
    fn parse_cell_with_annotation_and_initializer() {
        let _sheet = parser()
            .parse_str("sheet s { cell width: f64 = 1920.0; }")
            .unwrap();
    }

    /// Accepts a defaultable cell without an explicit initializer.
    #[test]
    fn parse_cell_annotation_only_uses_default() {
        let _sheet = parser().parse_str("sheet s { cell area: f64; }").unwrap();
    }

    /// Accepts an initialized cell without an annotation.
    #[test]
    fn parse_cell_initializer_infers_type() {
        let _sheet = parser().parse_str("sheet s { cell mode = 0i32; }").unwrap();
    }

    /// Reports an unknown cell annotation by name.
    #[test]
    fn parse_cell_unknown_type_is_error() {
        let result = parser().parse_str("sheet s { cell x: unknown_type; }");
        assert!(result.is_err());
        let err = result.expect_err("expected Err");
        let msg = err.message().to_lowercase();
        assert!(
            msg.contains("unknown type") || msg.contains("unknown_type"),
            "{msg}"
        );
    }

    /// Rejects an uninitialized leaf that has no registered default.
    #[test]
    fn parse_cell_missing_default_is_error() {
        /// Supplies a test leaf without default construction.
        #[derive(PartialEq, Clone, Debug)]
        struct NoDef(i32);
        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDef>("NoDef");
        let mut p = AdamParser::new(reg, OpLookup::new());
        let result = p.parse_str("sheet s { cell x: NoDef; }");
        assert!(result.is_err());
    }

    /// Evaluates typed arrays containing registered custom leaves.
    #[test]
    fn parse_cell_initializer_supports_registry_typed_arrays() {
        let mut reg = TypeRegistry::new();
        reg.register_no_default::<Custom>("Custom");
        let mut parser = AdamParser::new(reg, custom_lookup());

        let parsed = parser
            .parse_str(
                "sheet s { \
                    cell values: i32 = count([left, right]: [Custom]); \
                    cell empty: i32 = count([]: [Custom]); \
                    cell nested: i32 = count([[]: [Custom]]: [[Custom]]); \
                }",
            )
            .unwrap();

        let (values_id, _) = parsed.cell_names["values"];
        assert_eq!(*parsed.read::<i32>(values_id).unwrap(), 2);

        let (empty_id, _) = parsed.cell_names["empty"];
        assert_eq!(*parsed.read::<i32>(empty_id).unwrap(), 0);

        let (nested_id, _) = parsed.cell_names["nested"];
        assert_eq!(*parsed.read::<i32>(nested_id).unwrap(), 1);
    }

    /// Rejects an array annotation naming an unknown custom leaf.
    #[test]
    fn parse_cell_initializer_rejects_unknown_registry_typed_arrays() {
        let mut parser = AdamParser::new(TypeRegistry::new(), custom_lookup());
        let err = parser
            .parse_str("sheet s { cell values = []: [Custom]; }")
            .expect_err("unknown custom array annotations must fail");

        assert!(
            err.message().contains("unknown type `Custom`"),
            "got: {}",
            err.message()
        );
    }

    /// Retains persistent compound array values through cell initialization.
    #[test]
    fn parse_cell_initializer_preserves_persistent_array_cells() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let parsed = parser
            .parse_str("sheet s { cell values = []: [(i32, f64)]; }")
            .unwrap();
        /// Describes persistent arrays of the fixture's nested tuple elements.
        type Pairs = cel_runtime::ArrayOf<
            cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
            >,
        >;
        assert!(
            parsed
                .read_compound::<Pairs>(parsed.cell_names["values"].0)
                .unwrap()
                .is_empty()
        );
    }

    /// Records a source declaration as a source-kind runtime cell.
    #[test]
    fn parse_source_decl_registers_a_source_kind_cell() {
        let sheet = parser()
            .parse_str("sheet s { source width: i32 = 4; }")
            .unwrap();
        let (width, _) = sheet.cell_names["width"];
        assert_eq!(sheet.cell_kind(width), Some(adam_rs::CellKind::Source));
    }

    /// Rejects a source declaration lacking both annotation and initializer.
    #[test]
    fn parse_source_decl_requires_colon_or_equals() {
        let result = parser().parse_str("sheet s { source width; }");
        assert!(result.is_err());
    }

    /// Registers requirements attached to an ordinary cell.
    #[test]
    fn parse_cell_decl_with_a_require_block_attaches_requirements() {
        let sheet = parser()
            .parse_str("sheet s { cell x: i32 = 5 require { @positive x > 0; }; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert_eq!(sheet.cell_requirements(x).unwrap().len(), 1);
    }

    /// Registers requirements attached to a source cell.
    #[test]
    fn parse_source_decl_with_a_require_block_attaches_requirements() {
        let sheet = parser()
            .parse_str("sheet s { source x: i32 = 5 require { @positive x > 0; }; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert_eq!(sheet.cell_requirements(x).unwrap().len(), 1);
    }

    /// Registers a source cell's filter.
    #[test]
    fn parse_source_decl_with_a_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { source x: i32 = 5 filter 0..=10; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert!(sheet.filter_kind(x).is_some());
    }

    /// Registers an output cell's filter.
    #[test]
    fn parse_out_decl_with_a_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { cell width: i32 = 4; out area := width filter 0..=100; }")
            .unwrap();
        let area = sheet.output_names["area"];
        assert!(sheet.filter_kind(area).is_some());
    }

    /// Clamps writes through a filter without named dependencies.
    #[test]
    fn cell_filter_with_no_named_dependency_clamps_on_write() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell a: i32 filter if _ < 1 { 1 } else if _ > 100 { 100 } else { _ }; }",
            )
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(cell_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(cell_id).unwrap(), 100);
    }

    /// Evaluates a filter against the current value of its named dependency.
    #[test]
    fn cell_filter_referencing_a_cell_tracks_its_current_value() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { \
                     cell hi: i32 = 100; \
                     cell a: i32 filter if _ < 1 { 1 } else if _ > hi { hi } else { _ }; \
                 }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];

        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(hi_id, 10i32).unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    /// Deduplicates repeated filter dependencies without changing the result.
    #[test]
    fn cell_filter_referencing_the_same_value_twice_is_idempotent() {
        // Snap-to-grid: `_ - (_ % step)` — `_` referenced twice must denote the same value both
        // times, not two independent parameters.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell step: i32 = 10; cell a: i32 filter _ - (_ % step); }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(a_id, 27i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 20);
    }

    /// Attaches related source labels to a filter-dependency cycle.
    #[test]
    fn filter_dependency_cycle_reported_with_secondary_labels() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let ok_source = "sheet s {\n    cell b: i32 = 0;\n    cell a: i32 = 5;\n    relationship { b := a; }\n    cell c: i32 = 0 filter _ + b - b;\n}";
        assert!(parser.parse_str(ok_source).is_ok());

        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The parser only lets a filter reference already-declared cells, so the cycle becomes
        // visible when the later relationship adds the reverse dependency edge.
        let source = "sheet s {\n    cell b: i32 = 0;\n    cell a: i32 = 5 filter _ + b - b;\n    relationship { b := a; }\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(
            err.message(),
            "a filter or conditional depends on a cell it governs (dependency cycle)"
        );
        assert!(!err.secondary().is_empty());
    }

    /// Rejects a non-range filter that never references its candidate value.
    #[test]
    fn cell_filter_without_underscore_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter 1; }");
        assert!(err.is_err());
    }

    /// Rejects a filter whose result does not match the governed cell type.
    #[test]
    fn cell_filter_body_type_mismatch_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter _ > 0; }");
        assert!(err.is_err());
    }

    /// Rejects an unknown identifier in a filter.
    #[test]
    fn cell_filter_undeclared_identifier_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter _ + nope; }");
        assert!(err.is_err());
    }

    /// Clamps writes to an inclusive range.
    #[test]
    fn cell_filter_with_a_range_inclusive_body_clamps_on_write() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: i32 filter 0..=100; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        assert!(matches!(
            parsed.sheet.filter_kind(a_id),
            Some(adam_rs::FilterKind::Range { .. })
        ));
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);
    }

    /// Rejects invalid public erased inputs on every call without poisoning later bounds.
    /// Rejects malformed scalar range arguments at the public boundary.
    #[test]
    fn public_range_bounds_rejects_invalid_scalar_inputs() {
        let parsed = AdamParser::new(TypeRegistry::new(), OpLookup::new())
            .parse_str("sheet s { cell hi: f64 = 10.0; cell value: f64 filter 0.0..=hi; }")
            .unwrap();
        let Some(adam_rs::FilterKind::Range { bounds }) =
            parsed.sheet.filter_kind(parsed.cell_names["value"].0)
        else {
            panic!("expected range filter");
        };
        let invalid: [&[&dyn Any]; 4] = [&[&42u64], &[&1u8], &[], &[&10.0f64, &20.0f64]];
        for _ in 0..2 {
            for args in invalid {
                let (lo, hi) = bounds(&[&10.0f64]).unwrap();
                assert_eq!(*lo.downcast::<f64>().unwrap(), 0.0);
                assert_eq!(*hi.downcast::<f64>().unwrap(), 10.0);
                // The first mismatch has f64's size and alignment, so the pre-fix
                // regression fails by accepting it, without an undersized raw read.
                assert!(bounds(args).is_none(), "invalid scalar inputs accepted");
                let (_, hi) = bounds(&[&25.0f64]).unwrap();
                assert_eq!(*hi.downcast::<f64>().unwrap(), 25.0);
            }
        }
    }

    /// Checks complete compound shapes, carrier identity, arity, and recovery.
    /// Rejects malformed compound range arguments at the public boundary.
    #[test]
    fn public_range_bounds_rejects_invalid_compound_inputs() {
        use cel_runtime::{Cons, IntoCStackList, Leaf, Nil, OwnedValue};

        let parsed = AdamParser::new(TypeRegistry::new(), OpLookup::new())
            .parse_str(
                "sheet s { cell limits: (f64, f64) = (0.0, 10.0); \
                 cell value: f64 filter limits.0..=limits.1; }",
            )
            .unwrap();
        let Some(adam_rs::FilterKind::Range { bounds }) =
            parsed.sheet.filter_kind(parsed.cell_names["value"].0)
        else {
            panic!("expected range filter");
        };
        /// Describes the valid floating-point tuple used as range bounds.
        type Limits = Cons<Leaf<f64>, Cons<Leaf<f64>, Nil>>;
        /// Describes an incompatible tuple carrier supplied to the checked range boundary.
        type WrongLimits = Cons<Leaf<f64>, Cons<Leaf<u64>, Nil>>;
        let valid = CellValue(OwnedValue::new::<Limits>((0.0, 10.0).into_c_stack_list()));
        // Identical field sizes and valid f64 bits keep this regression safe pre-fix.
        let wrong_shape = CellValue(OwnedValue::new::<WrongLimits>(
            (0.0, 10.0f64.to_bits()).into_c_stack_list(),
        ));
        let invalid: [&[&dyn Any]; 4] = [&[&wrong_shape], &[&false], &[], &[&valid, &valid]];
        for _ in 0..2 {
            for args in invalid {
                let (lo, hi) = bounds(&[&valid]).unwrap();
                assert_eq!(*lo.downcast::<f64>().unwrap(), 0.0);
                assert_eq!(*hi.downcast::<f64>().unwrap(), 10.0);
                assert!(bounds(args).is_none(), "invalid compound inputs accepted");
                assert!(bounds(&[&valid]).is_some());
            }
        }
    }

    /// Accepts range filters without a candidate-value placeholder.
    #[test]
    fn cell_filter_range_does_not_require_underscore() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let result = parser.parse_str("sheet s { cell a: i32 filter 0..=100; }");
        assert!(result.is_ok());
    }

    /// Updates effective range bounds when their cell dependencies change.
    #[test]
    fn cell_filter_range_bounds_track_cell_dependencies_live() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell lo: i32 = 0; cell hi: i32 = 100; cell a: i32 filter lo..=hi; }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 100)));
        parsed.sheet.write(hi_id, 10i32).unwrap();
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 10)));
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    /// Applies inclusive range filtering to floating-point cells.
    #[test]
    fn cell_filter_range_with_float_cell_type_works() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: f64 filter 0.0..=100.0; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(a_id, 500.0f64).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<f64>(a_id).unwrap(), 100.0);
    }

    /// Rejects a range whose element type differs from the governed cell.
    #[test]
    fn cell_filter_range_with_mismatched_element_type_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser
            .parse_str("sheet s { cell a: f64 filter 0..=100; }")
            .expect_err("expected Err");
        assert!(
            err.message().contains("filter range bounds must be"),
            "{}",
            err.message()
        );
    }

    /// Reports range element mismatch even when a bound references the candidate.
    #[test]
    fn cell_filter_range_with_mismatched_element_type_and_underscore_is_still_a_range_error() {
        // Confirms the "range bounds must be" check fires unconditionally on a recognized
        // RangeInclusive-shaped output, even though `_` is referenced here (cast to `i32`, the
        // range's own element type, rather than left at the cell's declared `f64`) — it must not
        // be swallowed by the opaque `underscore_used` path.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser
            .parse_str("sheet s { cell a: f64 filter (_ as i32)..=100; }")
            .expect_err("expected Err");
        assert!(
            err.message().contains("filter range bounds must be"),
            "{}",
            err.message()
        );
    }

    /// Leaves range bounds unavailable when their expression evaluation fails.
    #[test]
    fn cell_filter_range_returns_none_instead_of_panicking_when_a_bound_expression_fails_to_evaluate()
     {
        // `100 / hi` is a registered CEL builtin (`/` on `i32`, see
        // `cel_parser::op_table::OpLookup`) that returns `Err` at runtime on division by zero
        // rather than panicking or wrapping — `parse_cell_filter` only validates this range
        // filter's *type* at parse time, so evaluating the bound expression with `hi == 0` must
        // still be handled at runtime. Before this fix, `FilterKind::Range::bounds` was
        // infallible and `parse_cell_filter`'s wrapping closure used `.expect(..)`, which would
        // panic here instead of degrading to `None`.
        // `hi` starts at a non-zero value so `add_filter`'s own immediate clamp of `a`'s default
        // value (0) succeeds during parsing (`0..=(100 / 1)`); the failing evaluation is
        // triggered afterwards, purely through `filter_range`, by writing `hi` to 0.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell hi: i32 = 1; cell a: i32 filter 0..=(100 / hi); }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];

        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 100)));

        // Puts the range expression's own evaluation into a failing state (division by zero)
        // without going through `add_filter`'s clamp path at all — this is exactly the "some
        // other write puts an endpoint's dependency into a bad state" scenario from the review
        // finding, and must degrade to `None` rather than panicking.
        parsed.sheet.write(hi_id, 0i32).unwrap();
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), None);
    }

    /// Keeps non-range filter expressions opaque.
    #[test]
    fn cell_filter_general_expression_still_compiles_to_opaque_kind() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let parsed = parser
            .parse_str("sheet s { cell a: i32 filter if _ < 0 { 0 } else { _ }; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        assert!(matches!(
            parsed.sheet.filter_kind(a_id),
            Some(adam_rs::FilterKind::Opaque)
        ));
    }

    /// Tracks changing bounds supplied by a tuple-valued cell.
    #[test]
    fn filter_tracks_a_tuple_typed_range_cell_dynamically() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { \
                     cell a_range: (i32, i32) = (1, 100); \
                     cell max: i32 = 100; \
                     relationship { a_range := (1, max); } \
                     cell a: i32 filter if _ < a_range.0 { a_range.0 } \
                         else if _ > a_range.1 { a_range.1 } else { _ }; \
                 }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (max_id, _) = parsed.cell_names["max"];

        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(max_id, 10i32).unwrap();
        parsed.sheet.propagate().unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    /// Accepts filtering a tuple-valued cell.
    #[test]
    fn cell_filter_on_a_tuple_typed_cell_is_supported() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let sheet = parser
            .parse_str("sheet s { cell a: (i32, i32) filter (_.0, _.1); }")
            .unwrap();
        assert!(sheet.filter_kind(sheet.cell_names["a"].0).is_some());
    }

    /// Attaches a parsed filter clause to its cell.
    #[test]
    fn parse_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { cell x: i32 = 0 filter 0..=10; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert!(sheet.filter_kind(x).is_some());
    }

    /// Rejects labels on filter clauses.
    #[test]
    fn parse_filter_clause_with_a_label_is_now_a_parse_error() {
        let result = parser().parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }");
        assert!(result.is_err());
    }

    /// Accepts multiple declarations in one sheet.
    #[test]
    fn parse_multiple_cells() {
        let _sheet = parser()
            .parse_str(
                r#"
            sheet image_resize {
                cell width:  f64 = 1920.0;
                cell height: f64 = 1080.0;
                cell area:   f64;
                cell mode:   i32 = 0;
            }
        "#,
            )
            .unwrap();
    }

    /// Evaluates a single-binding relationship.
    #[test]
    fn parse_relationship_with_a_single_binding() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 0;
                    relationship {
                        b := a;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (b_id, _) = sheet.cell_names["b"].clone();
        assert_eq!(*sheet.read::<i32>(b_id).unwrap(), 2);
    }

    /// Infers relationship dependencies from referenced cell identifiers.
    #[test]
    fn parse_relationship_deduces_inputs_from_referenced_identifiers() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 3;
                    cell c: i32 = 0;
                    relationship {
                        c := a * b;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (c_id, _) = sheet.cell_names["c"].clone();
        assert_eq!(*sheet.read::<i32>(c_id).unwrap(), 6);
    }

    /// Verifies member labels never become dependencies or resolve same-named cells.
    /// Excludes member labels from relationship dependencies.
    #[test]
    fn member_labels_are_not_cell_dependencies() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _| {
            if matches!(name, "x" | "width") && arity == 1 {
                segment.op1(|value: f64| value * 2.0)?;
                return Ok(true);
            }
            Ok(false)
        });
        let mut sheet = AdamParser::new(TypeRegistry::new(), lookup)
            .parse_str(
                "sheet s { cell input: f64 = 3.0; cell x: bool = true; \
                 cell width: bool = false; cell result: f64 = 0.0; \
                 relationship { result := input.x().width; } }",
            )
            .unwrap();
        let input = sheet.cell_names["input"].0;
        let result = sheet.cell_names["result"].0;
        let (&(relationship, method), _) = sheet.method_spans.iter().next().unwrap();
        assert_eq!(
            sheet.method_inputs(relationship, method),
            Some([input].as_slice())
        );
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 12.0);
        sheet.write(input, 5.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 20.0);
    }

    /// Tracks only value expressions in a record-member binding.
    /// Excludes record labels and member names from cell dependency resolution.
    #[test]
    fn field_labels_and_member_names_are_not_cell_dependencies() {
        use cel_parser::{OperationSignature, RecordField, RecordSchema, ResolvedLeafType};
        use cel_runtime::ValueType;
        /// A record independent of any geometry library.
        #[derive(Clone, Debug, PartialEq)]
        struct TestPoint {
            x: f64,
            y: f64,
        }
        let point = ResolvedLeafType::new("TestPoint", ValueType::leaf::<TestPoint>());
        let scalar = ResolvedLeafType::new("f64", ValueType::leaf::<f64>());
        let mut lookup = OpLookup::new();
        lookup
            .register_record(RecordSchema {
                ty: point.clone(),
                fields: vec![
                    RecordField {
                        name: "x",
                        ty: scalar.clone().into(),
                    },
                    RecordField {
                        name: "y",
                        ty: scalar.clone().into(),
                    },
                ],
                emit: Box::new(|s, order, _, _| {
                    assert_eq!(order, [0, 1]);
                    s.op2(|x: f64, y: f64| TestPoint { x, y })
                }),
            })
            .unwrap();
        lookup
            .register_operation(OperationSignature {
                name: "x",
                operands: vec![point.into()],
                result: scalar.into(),
                emit: Box::new(|s, _| s.op1(|p: TestPoint| p.x)),
            })
            .unwrap();
        let mut sheet = AdamParser::new(TypeRegistry::new(), lookup)
            .parse_str(
                "sheet s { cell a: f64 = 2.0; cell b: f64 = 3.0; \
             cell x: bool = false; cell y: bool = false; cell TestPoint: bool = true; \
             cell result: f64 = 0.0; \
             relationship { result := TestPoint { x: a, y: b }.x; } }",
            )
            .unwrap();
        let a = sheet.cell_names["a"].0;
        let b = sheet.cell_names["b"].0;
        let result = sheet.cell_names["result"].0;
        let (&(relationship, method), _) = sheet.method_spans.iter().next().unwrap();
        assert_eq!(
            sheet.method_inputs(relationship, method),
            Some([a, b].as_slice())
        );
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 2.0);
        sheet.write(a, 7.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 7.0);
    }

    /// Executes the planner-selected direction of a reversible relationship.
    #[test]
    fn parse_relationship_with_multiple_bindings_lets_the_planner_pick_a_direction() {
        // `c` is declared first (and so has the weakest cell strength — `adam_rs`'s
        // planner processes cells in *descending* strength order, preferentially
        // leaving the strongest cells as sources, so the earliest-declared cell is the
        // one left to be computed): with `a`/`b` fixed as sources, the only
        // self-consistent choice among these three mutually-referencing bindings is to
        // compute `c := a * b`.
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell c: i32 = 0;
                    cell a: i32 = 2;
                    cell b: i32 = 3;
                    relationship {
                        c := a * b;
                        a := c / b;
                        b := c / a;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (c_id, _) = sheet.cell_names["c"].clone();
        assert_eq!(*sheet.read::<i32>(c_id).unwrap(), 6);
    }

    /// Attributes inconsistent method cell sets to their relationship block.
    #[test]
    fn mismatched_method_cells_error_spans_the_relationship_block_not_the_sheet() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source = "sheet s {\n    cell a: i32;\n    cell b: i32;\n\n    relationship {\n        a := b;\n        b := 42;\n    }\n}";
        let err = parser.parse_str(source).err().unwrap();
        // The mismatched binding (`b := 42;`, method index 1) is on line 7; the sheet's
        // opening line (1) must not be reported instead. `ParseError::span()` returns a
        // `proc_macro2::Span`, so `.start()` is a method call here, not a field access (unlike
        // `cel_parser::SourceSpan`, whose `start`/`end` are public `LineColumn` fields).
        assert_eq!(err.span().start().line, 7);
    }

    /// Retains method source spans after successful parsing.
    #[test]
    fn successful_relationship_parse_populates_method_spans() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source = "sheet s {\n    cell a: i32;\n    cell b: i32;\n\n    relationship {\n        a := b;\n    }\n}";
        let parsed = parser.parse_str(source).unwrap();
        assert_eq!(parsed.method_spans.len(), 1);
        let ((_, idx), span) = parsed.method_spans.iter().next().unwrap();
        assert_eq!(*idx, 0);
        assert_eq!(span.start.line, 6);
    }

    /// Retains source spans for runtime relationships and cells.
    #[test]
    fn parse_populates_relationship_and_cell_spans() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let parsed = parser
            .parse_str("sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; } }")
            .unwrap();
        assert_eq!(parsed.cell_spans.len(), 2);
        assert_eq!(parsed.relationship_spans.len(), 1);
        // every declared cell id has a span
        for (_, (id, _)) in &parsed.cell_names {
            assert!(parsed.cell_spans.contains_key(id));
        }
    }

    /// Resolves a cycle diagnostic to ordered participating source spans.
    #[test]
    fn locate_error_resolves_a_cycle_to_multiple_ordered_spans() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell x: i32 = 0; cell y: i32 = 0; relationship { x := y + 1i32; } \
                 relationship { y := x + 1i32; } }",
            )
            .unwrap();
        let err = parsed.propagate().unwrap_err();
        assert!(matches!(err, adam_rs::Error::Cycle { .. }));
        let located = parsed.locate_error(&err);
        // both relationship blocks resolve to spans
        let rel_spans = located.len();
        assert!(
            rel_spans >= 2,
            "expected >=2 spans, got {rel_spans}: {located:?}"
        );
    }

    /// Labels both bindings that produce the same output set.
    #[test]
    fn duplicate_output_set_error_underlines_both_bindings() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source =
            "sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; b := a + 1i32; } }";
        let err = parser.parse_str(source).unwrap_err();
        let out =
            err.format_rustc_style(source, "t.adm2", 1, &annotate_snippets::Renderer::plain());
        // Primary caret on the later binding, secondary carets on the earlier binding and the
        // shared output cell's own declaration.
        assert!(out.contains("outputs"), "{out}"); // message text
        assert!(out.contains("collides with this earlier method"), "{out}");
        assert!(
            out.contains("output cell `b` is claimed more than once"),
            "{out}"
        );
    }

    /// Rejects an undeclared relationship output.
    #[test]
    fn parse_binding_undeclared_output_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                relationship {
                    missing := a;
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    /// Executes destructuring against the declared output tuple shapes.
    #[test]
    fn parse_binding_multi_output_tuple_matches_existing_tuple_shape_rules() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    cell y: i32 = 0;
                    relationship {
                        (x, y) := (w, w * 2);
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (x_id, _) = sheet.cell_names["x"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(x_id).unwrap(), 4);
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 8);
    }

    /// Rejects destructuring with the wrong number of tuple elements.
    #[test]
    fn parse_binding_multi_output_arity_mismatch_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell w: i32 = 4;
                cell x: i32 = 0;
                cell y: i32 = 0;
                relationship {
                    (x, y) := w;
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    /// Rejects multi-output targets without tuple parentheses.
    #[test]
    fn parse_binding_multi_output_without_parens_is_a_parse_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell w: i32 = 4;
                cell x: i32 = 0;
                cell y: i32 = 0;
                relationship {
                    x, y := (w, w * 2);
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    /// Extracts the sole value of a singleton tuple binding.
    #[test]
    fn parse_binding_single_element_tuple_destructure_extracts_the_element() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    relationship {
                        (x,) := (w,);
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (x_id, _) = sheet.cell_names["x"].clone();
        assert_eq!(*sheet.read::<i32>(x_id).unwrap(), 4);
    }

    /// Rejects a parenthesized binding name instead of treating it as a direct bind.
    /// Rejects a parenthesized binding target lacking the singleton comma.
    #[test]
    fn parse_binding_single_parenthesized_identifier_without_comma_is_an_error() {
        let result = parser().parse_str(
            r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    relationship {
                        (x) := w;
                    }
                }
            "#,
        );
        assert!(result.is_err());
    }

    /// Evaluates an explicitly typed output without requirements.
    #[test]
    fn parse_out_with_direct_initializer_and_no_require_block() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b;
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let cell_id = sheet.output_names["area"];
        assert_eq!(*sheet.sheet.read::<i32>(cell_id).unwrap(), 12);
    }

    /// Evaluates an output with an inferred type.
    #[test]
    fn parse_out_with_no_type_annotation_infers_from_initializer() {
        let sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    out doubled := a * 2;
                }
            "#,
            )
            .unwrap();
        let (_, shape) = sheet.cell_names["doubled"].clone();
        assert_eq!(
            shape,
            crate::type_registry::TypeShape::Named(std::any::TypeId::of::<i32>())
        );
    }

    /// Registers explicitly named output requirements.
    #[test]
    fn parse_out_with_a_require_block_registers_named_requirements() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b require {
                        @positive area > 0;
                        @small area < 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert!(sheet.sheet.cell_requirements(output_id).unwrap().len() == 2);
        assert!(
            sheet
                .sheet
                .violated_requirements(output_id)
                .next()
                .is_none()
        );
    }

    /// Reports a violated output requirement.
    #[test]
    fn parse_out_require_block_requirement_can_violate() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b require {
                        @too_small area > 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert_eq!(sheet.sheet.violated_requirements(output_id).count(), 1);
    }

    /// Rejects a requirement whose expression is not Boolean.
    #[test]
    fn parse_requirement_non_bool_body_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 3;
                out x: i32 := a require {
                    @bad a;
                };
            }
        "#,
        );
        assert!(result.is_err());
    }

    /// Accepts an outer documentation comment on a cell.
    #[test]
    fn parses_a_sheet_with_an_outer_doc_comment_on_a_cell() {
        let parsed = parser()
            .parse_str("sheet s {\n    /// the total\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(parsed.cell_names.len(), 1);
    }

    /// Accepts sheet-level inner documentation.
    #[test]
    fn parses_a_sheet_with_an_inner_doc_comment() {
        let parsed = parser()
            .parse_str("//! module docs\nsheet s {\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(parsed.cell_names.len(), 1);
    }

    /// Accepts documentation comments across supported declaration kinds.
    #[test]
    fn parses_a_sheet_with_doc_comments_on_every_declaration_kind() {
        let source = "//! module docs\nsheet s {\n    /// a cell\n    cell x: i32 = 1;\n\n    /// another cell\n    cell y: i32 = 2;\n\n    /// a relationship\n    relationship { y := x; }\n}";
        let parsed = parser().parse_str(source).unwrap();
        assert_eq!(parsed.cell_names.len(), 2);
    }

    /// Rejects an undeclared relationship input.
    #[test]
    fn parse_binding_undeclared_input_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: f64 = 1.0;
                relationship { x := bogus; }
            }
        "#,
        );
        assert!(result.is_err());
        let err = result.expect_err("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("bogus") || msg.contains("undefined"), "{msg}");
    }

    /// Rejects a method result that conflicts with its output annotation.
    #[test]
    fn parse_method_output_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: f64 = 0.0;
                cell n: i32 = 0;
                relationship { n := x; }
            }
        "#,
        );
        assert!(result.is_err(), "f64 body for i32 output must be an error");
    }

    /// Rejects a scalar multi-output method with the wrong tuple arity.
    #[test]
    fn parse_method_output_tuple_arity_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: i32 = 2;
                cell x: i32;
                cell y: i32;
                cell z: i32;
                relationship { (x, y, z) := (a + b, a - b); }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "2-tuple body for 3 declared outputs must be an error"
        );
        let err = result.expect_err("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("arity"), "{msg}");
    }

    /// Names the registered leaf in a method result mismatch.
    #[test]
    fn parse_method_output_type_mismatch_names_the_registered_type() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: String = "x";
                cell x: i32;
                relationship { x := a; }
            }
        "#,
        );
        let err = result.expect_err("a String body for an i32 output must be an error");
        let msg = err.message();
        assert!(msg.contains("got `String`"), "{msg}");
        assert!(!msg.contains("::"), "no Rust type path may leak: {msg}");
    }

    /// Names the registered leaf in a destructured result mismatch.
    #[test]
    fn parse_method_destructured_output_type_mismatch_names_the_registered_type() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: String = "x";
                cell b: i32 = 1;
                cell x: i32;
                cell y: i32;
                relationship { (x, y) := (a, b); }
            }
        "#,
        );
        let err = result.expect_err("a String tuple element for an i32 output must be an error");
        let msg = err.message();
        assert!(msg.contains("got `String`"), "{msg}");
        assert!(!msg.contains("::"), "no Rust type path may leak: {msg}");
    }

    /// Rejects a tuple method result containing the wrong element type.
    #[test]
    fn parse_method_output_tuple_element_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: f64 = 2.0;
                cell x: i32;
                cell y: i32;
                relationship { (x, y) := (a, b); }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "f64 tuple element for an i32 output must be an error"
        );
        let err = result.expect_err("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("type mismatch"), "{msg}");
    }

    /// Rejects a tuple body assigned to a scalar output.
    #[test]
    fn parse_method_single_output_rejects_tuple_body() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 1;
                cell y: i32;
                relationship { y := (x,); }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "1-tuple body for a single declared output must be an error"
        );
    }

    /// Executes a method with one tuple-valued output.
    #[test]
    fn parse_method_single_tuple_typed_output() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    cell pair: (i32, i32);
                    relationship { pair := (a, b); }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (cell_id, _) = sheet.cell_names["pair"].clone();
        let value = sheet
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
        assert_eq!((a, b), (3, 4));
    }

    /// Executes a method combining tuple-valued and scalar outputs.
    #[test]
    fn parse_method_tuple_typed_output_among_several() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    cell pair: (i32, i32);
                    cell extra: i32;
                    relationship { (pair, extra) := ((a, b), a); }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (pair_id, _) = sheet.cell_names["pair"].clone();
        let (extra_id, _) = sheet.cell_names["extra"].clone();
        let pair = sheet
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            >>(pair_id)
            .unwrap();
        let (a, b) = (pair.1, pair.0.1);
        assert_eq!((a, b), (3, 4));
        assert_eq!(*sheet.read::<i32>(extra_id).unwrap(), 3);
    }

    /// Reads tuple input elements through field indexing.
    #[test]
    fn parse_method_with_tuple_typed_input_supports_field_indexing() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell pair: (i32, i32) = (10, 20);
                    cell sum: i32;
                    relationship { sum := pair.0 + pair.1; }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (sum_id, _) = sheet.cell_names["sum"].clone();
        assert_eq!(*sheet.read::<i32>(sum_id).unwrap(), 30);
    }

    /// Rejects a method result with a different tuple shape.
    #[test]
    fn parse_method_tuple_output_shape_mismatch_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: f64 = 2.0;
                cell pair: (i32, i32);
                relationship { pair := (a, b); }
            }
        "#,
        );
        assert!(result.is_err());
    }

    /// Checks both results of a scalar multi-output method.
    #[test]
    fn existing_multi_output_scalar_methods_still_work_unchanged() {
        // Regression: today's N-scalar-outputs mechanism must still behave identically after the
        // CompiledOutputs refactor.
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    cell sum: i32;
                    cell diff: i32;
                    relationship { (sum, diff) := (a + b, a - b); }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (sum_id, _) = sheet.cell_names["sum"].clone();
        let (diff_id, _) = sheet.cell_names["diff"].clone();
        assert_eq!(*sheet.read::<i32>(sum_id).unwrap(), 7);
        assert_eq!(*sheet.read::<i32>(diff_id).unwrap(), -1);
    }

    /// Executes a method producing the empty tuple.
    #[test]
    fn parse_method_single_empty_tuple_typed_output() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell x: i32 = 1;
                    cell nothing: ();
                    // `()` alone references no cell, and adam_rs rejects a method with no
                    // inputs -- reference `x` via an if/else whose branches both still
                    // evaluate to `()`, so the binding has a deduced input.
                    relationship { nothing := if x > 0 { () } else { () }; }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (cell_id, _) = sheet.cell_names["nothing"].clone();
        sheet.read_compound::<cel_runtime::Nil>(cell_id).unwrap();
    }

    /// Executes a conditional declaration.
    #[test]
    fn parse_conditional_decl() {
        let _sheet = parser()
            .parse_str(
                r#"
            sheet image_resize {
                cell width:  f64 = 1920.0;
                cell height: f64 = 1080.0;
                cell ratio:  f64 = 1.0;
                cell mode:   i32 = 0;
                conditional mode {
                    0i32 => {
                        relationship { height := width; }
                    }
                    1i32 => {
                        relationship { height := width * ratio; }
                    }
                    _ => {
                        relationship { height := width; }
                    }
                }
            }
        "#,
            )
            .unwrap();
    }

    /// Updates both relationships in an active branch after independent input edits.
    #[test]
    fn parse_conditional_branch_with_multiple_relationships() {
        let mut sheet = parser()
            .parse_str(
                r#"
            sheet s {
                cell mode: i32 = 0;
                cell a: f64 = 2.0;
                cell b: f64 = 3.0;
                cell c: f64;
                cell d: f64 = 4.0;
                cell e: f64 = 5.0;
                cell f: f64;
                conditional mode {
                    0i32 => {
                        relationship { c := a * b; }
                        relationship { f := d * e; }
                    }
                }
            }
        "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let c = sheet.cell_names["c"].0;
        let f = sheet.cell_names["f"].0;
        assert_eq!(*sheet.read::<f64>(c).unwrap(), 6.0);
        assert_eq!(*sheet.read::<f64>(f).unwrap(), 20.0);
        let a = sheet.cell_names["a"].0;
        sheet.write(a, 7.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(c).unwrap(), 21.0);
        assert_eq!(*sheet.read::<f64>(f).unwrap(), 20.0);
        let d = sheet.cell_names["d"].0;
        sheet.write(d, 6.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(c).unwrap(), 21.0);
        assert_eq!(*sheet.read::<f64>(f).unwrap(), 30.0);
    }

    /// Rejects a conditional-arm binding outside a relationship block.
    #[test]
    fn conditional_branch_bare_binding_without_relationship_wrapper_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 0;
                conditional x { 0i32 => { x := x; } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "a conditional_branch body now requires relationship_decl, not a bare binding"
        );
    }

    /// Rejects an unknown conditional subject cell.
    #[test]
    fn conditional_undeclared_match_cell_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 0;
                conditional bogus { 0i32 => { relationship { x := x; } } }
            }
        "#,
        );
        assert!(result.is_err());
        let err = result.expect_err("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("bogus") || msg.contains("undeclared"), "{msg}");
    }

    /// Rejects a branch key with the wrong subject type.
    #[test]
    fn conditional_branch_literal_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: i32 = 0;
                cell x:    f64 = 0.0;
                conditional mode { 1.0 => { relationship { x := x; } } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "float literal for i32 match cell must be an error"
        );
    }

    /// Selects the branch matching a negative literal key.
    #[test]
    fn conditional_branch_negative_literal_key_matches() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: i32 = -1;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional mode {
                        -1i32 => { relationship { y := x; } }
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    /// Rejects a negated unsigned branch key.
    #[test]
    fn conditional_branch_negating_unsigned_literal_key_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: u32 = 0;
                cell x: i32 = 1;
                cell y: i32 = 0;
                conditional mode {
                    -1u32 => { relationship { y := x; } }
                }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "negating an unsigned literal must be a parse error"
        );
    }

    /// Executes a default-only conditional on a tuple-valued subject.
    #[test]
    fn conditional_with_tuple_typed_match_cell_and_default_only_still_works() {
        // A tuple isn't a `literal_pattern`, so a tuple-typed match cell can no longer pair with
        // a named branch key — only the `_` default branch remains reachable.
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: (i32, i32) = (0, 0);
                    cell x: f64 = 1.0;
                    cell y: f64;
                    conditional mode {
                        _ => { relationship { y := x * 2.0; } }
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<f64>(y_id).unwrap(), 2.0);
    }

    /// Rejects a tuple literal as a conditional branch key.
    #[test]
    fn conditional_branch_tuple_literal_key_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: (i32, i32) = (0, 0);
                cell x: f64 = 1.0;
                cell y: f64;
                conditional mode {
                    (0, 0) => { relationship { y := x; } }
                    _ => { relationship { y := x * 2.0; } }
                }
            }
        "#,
        );
        assert!(result.is_err(), "tuple branch key must be a parse error");
    }

    /// Reacts to changes in either cell of a Boolean conditional subject.
    #[test]
    fn conditional_on_a_two_cell_boolean_expression_activates_and_reacts_to_writes() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: bool = false;
                    cell b: bool = false;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional a && b {
                        true => { relationship { y := x; } }
                    }
                }
            "#,
            )
            .unwrap();
        let (a_id, _) = sheet.cell_names["a"].clone();
        let (b_id, _) = sheet.cell_names["b"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();

        sheet.write(a_id, true).unwrap();
        sheet.write(b_id, false).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 0);

        sheet.write(b_id, true).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    /// Evaluates a conditional subject with repeated references to one cell.
    #[test]
    fn conditional_expression_referencing_the_same_cell_twice_compiles_and_evaluates() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: bool = true;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional a && a {
                        true => { relationship { y := x; } }
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    /// Evaluates a bare cell identifier as a conditional subject.
    #[test]
    fn conditional_bare_identifier_match_subject_still_works() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: i32 = 0;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional mode {
                        1i32 => { relationship { y := x; } }
                    }
                }
            "#,
            )
            .unwrap();
        let (mode_id, _) = sheet.cell_names["mode"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();

        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 0);

        sheet.write(mode_id, 1_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    /// Verifies runtime conditional headers accept grouped records and deduce field dependencies.
    #[test]
    fn adam_registered_match_header_reserves_braces() {
        let mut lookup = OpLookup::new();
        lookup
            .register_record(cel_parser::RecordSchema {
                ty: cel_parser::ResolvedLeafType::new(
                    "Gate",
                    cel_runtime::ValueType::leaf::<bool>(),
                ),
                fields: vec![cel_parser::RecordField {
                    name: "value",
                    ty: cel_parser::ResolvedLeafType::new(
                        "bool",
                        cel_runtime::ValueType::leaf::<bool>(),
                    )
                    .into(),
                }],
                emit: Box::new(|segment, _, _, _| segment.op1(|value: bool| value)),
            })
            .unwrap();
        let mut parser = AdamParser::new(TypeRegistry::new(), lookup);
        let mut sheet = parser
            .parse_str(
                "sheet s {
                cell flag: bool = true;
                cell input: i32 = 7;
                cell output: i32 = 0;
                conditional (Gate { value: flag }) {
                    true => { relationship { output := if flag { input } else { 0 }; } }
                    _ => { relationship { output := 0; } }
                }
            }",
            )
            .unwrap();
        let output = sheet.cell_names["output"].0;
        let flag = sheet.cell_names["flag"].0;
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
        sheet.write(flag, false).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 0);
    }

    /// Rejects a bare identifier as a branch key.
    #[test]
    fn conditional_branch_bare_identifier_key_is_error() {
        // A bare identifier isn't a `literal_pattern` either, even when it resolves (as a
        // client-registered constant would) to a value of the match subject's own type.
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: i32 = 0;
                cell x: i32 = 1;
                cell y: i32 = 0;
                conditional mode {
                    mode => { relationship { y := x; } },
                }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "bare identifier branch key must be a parse error"
        );
    }

    /// Rejects an unknown identifier inside a conditional subject expression.
    #[test]
    fn conditional_expression_referencing_an_undeclared_identifier_is_a_parse_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: bool = true;
                conditional a && nope {
                    true => { relationship { a := a; } }
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    /// Rejects a cell literal that conflicts with its annotation.
    #[test]
    fn parse_cell_literal_type_mismatch_is_error() {
        // Float literal for an i32 cell should be a parse error.
        let result = parser().parse_str("sheet s { cell x: i32 = 1.0; }");
        assert!(
            result.is_err(),
            "float literal for i32 annotation must be an error"
        );
    }

    /// Computes floating-point results in both directions after a strength-changing edit.
    #[test]
    fn parse_and_propagate_sheet() {
        let mut p = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut sheet = p
            .parse_str(
                r#"
            sheet image_resize {
                cell area:   f64;
                cell width:  f64 = 4.0;
                cell height: f64 = 3.0;
                relationship {
                    area := width * height;
                    width := area / height;
                    height := area / width;
                }
            }
        "#,
            )
            .unwrap();

        sheet.propagate().unwrap();
        let area = sheet.cell_names["area"].0;
        let width = sheet.cell_names["width"].0;
        assert_eq!(*sheet.read::<f64>(area).unwrap(), 12.0);
        sheet.write(area, 24.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(width).unwrap(), 8.0);
    }

    /// Rejects duplicate cell declarations.
    #[test]
    fn parse_duplicate_cell_is_error() {
        let result = parser().parse_str("sheet s { cell x: i32; cell x: f64; }");
        match result {
            Ok(_) => panic!("expected error for duplicate cell name"),
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("duplicate cell `x`"),
                    "error mentions duplicate name: {msg}"
                );
            }
        }
    }

    /// Preserves cell declaration order in returned name metadata.
    #[test]
    fn parse_str_returns_cell_names_in_declaration_order() {
        let parsed = parser()
            .parse_str("sheet s { cell z: i32 = 1; cell a: i32 = 2; cell m: i32 = 3; }")
            .unwrap();
        let names: Vec<&str> = parsed.cell_names.keys().map(String::as_str).collect();
        assert_eq!(names, vec!["z", "a", "m"]);
    }

    /// Exposes runtime propagation through a parsed sheet.
    #[test]
    fn parsed_sheet_derefs_to_sheet_for_propagate() {
        let mut parsed = parser().parse_str("sheet s { cell x: i32 = 1; }").unwrap();
        // Deref/DerefMut must make Sheet's methods directly callable.
        parsed.propagate().unwrap();
    }

    /// Propagates an explicitly typed output.
    #[test]
    fn parse_out_with_explicit_type_propagates_correctly() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 4.0;
                    cell height: f64 = 3.0;
                    out area: f64 := width * height;
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let cell_id = *sheet.output_names.get("area").expect("area registered");
        assert_eq!(*sheet.read::<f64>(cell_id).unwrap(), 12.0);
    }

    /// Formats a stored tuple output using its concrete value.
    #[test]
    fn parse_out_with_tuple_type_value_debug_formats_correctly() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell x: i32 = 3;
                    out pair: (i32, i32) := (x, x);
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let cell_id = *sheet.output_names.get("pair").unwrap();
        let value = sheet.read::<CellValue>(cell_id).unwrap();
        assert_eq!(format!("{value:?}"), "(3, 3)");
    }

    /// Infers an output's runtime type from its writer.
    #[test]
    fn parse_out_with_no_annotation_infers_type_from_writer_body() {
        let sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 4.0;
                    out doubled := width + width;
                }
            "#,
            )
            .unwrap();
        let (_, shape) = sheet.cell_names.get("doubled").unwrap().clone();
        assert_eq!(shape, TypeShape::Named(std::any::TypeId::of::<f64>()));
    }

    /// Rejects an output writer that conflicts with its annotation.
    #[test]
    fn parse_out_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                out area: i32 := width;
            }
        "#,
        );
        assert!(
            result.is_err(),
            "f64 body for an i32 annotation must be an error"
        );
    }

    /// Distinguishes valid and violated output requirements.
    #[test]
    fn parse_out_with_requirements_reports_output_valid_and_violated() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 4.0;
                    cell height: f64 = 3.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        @max_area width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert!(sheet.cell_requirements_valid(output_id));
        assert_eq!(sheet.violated_requirements(output_id).count(), 0);
    }

    /// Reports output requirement violations after propagation.
    #[test]
    fn parse_out_requirement_violation_is_reported_after_propagate() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 40.0;
                    cell height: f64 = 30.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        @max_area width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert!(!sheet.cell_requirements_valid(output_id));
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    /// Records metadata only for explicitly labeled requirements.
    #[test]
    /// Resolves explicit labels by returned IDs while preserving unnamed checks and order.
    fn requirement_names_are_metadata_for_explicit_labels_only() {
        let mut parsed = parser()
            .parse_str(
                "sheet s {
                    cell x: i32 = 1 require { @same x > 0i32; x > 0i32; x < 9i32; };
                    source y: i32 = 2 require { @same y > 0i32; y > 0i32; };
                    out z := x require { @last z < 0i32; z < 0i32; @first z > 9i32; };
                }",
            )
            .unwrap();
        let x = parsed.cell_names["x"].0;
        let y = parsed.cell_names["y"].0;
        let z = parsed.output_names["z"];
        assert_eq!(parsed.requirement_names.len(), 4);
        for cell in [x, y] {
            let ids = parsed.cell_requirements(cell).unwrap();
            assert_eq!(parsed.requirement_names[&ids[0]], "same");
            assert!(!parsed.requirement_names.contains_key(&ids[1]));
        }
        parsed.propagate().unwrap();
        let violated: Vec<_> = parsed.violated_requirements(z).collect();
        assert_eq!(violated, parsed.cell_requirements(z).unwrap());
        assert_eq!(parsed.requirement_names[&violated[0]], "last");
        assert!(!parsed.requirement_names.contains_key(&violated[1]));
        assert_eq!(parsed.requirement_names[&violated[2]], "first");
    }

    /// Rejects repeated requirement labels within each declaration kind.
    #[test]
    /// Rejects repeated explicit labels locally for every declaration kind.
    fn duplicate_requirement_labels_are_rejected_per_declaration() {
        for declaration in [
            "cell x: i32 = 1",
            "source x: i32 = 1",
            "cell y: i32 = 1; out x := y",
        ] {
            let source =
                format!("sheet s {{ {declaration} require {{ @dup x > 0i32; @dup x < 9i32; }}; }}");
            assert!(parser().parse_str(&source).is_err(), "{source}");
        }
    }

    /// Enforces an unlabeled output requirement.
    #[test]
    fn parse_out_with_an_unlabeled_requirement_is_enforced() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 40.0;
                    cell height: f64 = 30.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    /// Enforces an unlabeled requirement whose body is a bare identifier.
    #[test]
    fn parse_out_with_a_bare_identifier_unlabeled_requirement_is_enforced() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell is_valid: bool = false;
                    out area: i32 := 4 require {
                        is_valid;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    /// Accepts an output cell referenced by another expression.
    #[test]
    fn parse_out_cell_referenced_elsewhere_succeeds() {
        // `out` cells are no longer terminal (see `adam_rs::CellKind`): referencing an `out`
        // cell as another relationship's input is legal.
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                out area: f64 := width * height;
                relationship { width := area; }
            }
        "#,
        );
        assert!(
            result.is_ok(),
            "referencing an out cell as another relationship's input must not be an error"
        );
    }

    /// Infers structural array outputs without registering the array carrier.
    #[test]
    fn parse_out_infers_structural_array_without_registration() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell dummy: i32 = 0;
                out x := []: [i32];
            }
        "#,
        );
        let mut sheet = result.unwrap();
        sheet.propagate().unwrap();
        assert!(
            sheet
                .read_compound::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>>(
                    sheet.cell_names["x"].0
                )
                .unwrap()
                .is_empty()
        );
    }

    /// Empty tuple writers execute their initializer and propagate runtime errors.
    /// Executes an output writer even when its result is the empty tuple.
    #[test]
    fn empty_tuple_output_executes_its_writer() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _| {
            if name == "bad" && arity == 0 {
                segment.op0r(|| -> anyhow::Result<i32> { anyhow::bail!("unit writer failed") });
                return Ok(true);
            }
            Ok(false)
        });
        let mut sheet = AdamParser::new(TypeRegistry::new(), lookup)
            .parse_str("sheet s { cell input: i32 = 0; cell nothing: (); relationship { nothing := (input, bad, ()).2; } }")
            .unwrap();
        let error = sheet.propagate().unwrap_err();
        assert!(error.to_string().contains("unit writer failed"), "{error}");
    }

    /// Accepts an output cell as a conditional subject.
    #[test]
    fn parse_out_cell_referenced_in_conditional_succeeds() {
        // `out` cells are no longer terminal (see `adam_rs::CellKind`): referencing an `out`
        // cell as a relationship input inside a conditional branch is legal.
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                cell mode: i32 = 0;
                out area: f64 := width * height;
                conditional mode {
                    0i32 => { relationship { width := area; } }
                }
            }
        "#,
        );
        assert!(
            result.is_ok(),
            "referencing an out cell as a conditional branch relationship's input must not be an error"
        );
    }

    /// Initializes a cell with an explicitly declared tuple shape.
    #[test]
    fn parse_cell_with_explicit_tuple_type_and_initializer() {
        let mut p = parser();
        let parsed = p
            .parse_str("sheet s { cell a: (i32, f64) = (1, 2.5); }")
            .unwrap();
        let (cell_id, shape) = parsed.cell_names["a"].clone();
        assert_eq!(
            shape,
            TypeShape::Tuple(vec![
                TypeShape::Named(std::any::TypeId::of::<i32>()),
                TypeShape::Named(std::any::TypeId::of::<f64>()),
            ])
        );
        let value = parsed
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
        assert_eq!((a, b), (1, 2.5));
    }

    /// Constructs recursive defaults for tuple-typed cells.
    #[test]
    fn parse_cell_with_tuple_type_and_no_initializer_uses_recursive_default() {
        let parsed = parser()
            .parse_str("sheet s { cell a: (i32, f64); }")
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"].clone();
        let value = parsed
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
        assert_eq!((a, b), (0, 0.0));
    }

    /// Rejects an initializer with the wrong tuple arity.
    #[test]
    fn parse_cell_with_tuple_initializer_arity_mismatch_is_an_error() {
        let result = parser().parse_str("sheet s { cell a: (i32, f64, i32) = (1, 2.5); }");
        assert!(result.is_err());
    }

    /// Stores and reads a nested tuple cell.
    #[test]
    fn parse_cell_with_nested_tuple_type_round_trips() {
        let parsed = parser()
            .parse_str("sheet s { cell a: (i32, (f64, String)) = (1, (2.5, \"x\")); }")
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"].clone();
        /// Describes the fixture's nested tuple value.
        type Inner = cel_runtime::Cons<
            cel_runtime::Leaf<f64>,
            cel_runtime::Cons<cel_runtime::Leaf<String>, cel_runtime::Nil>,
        >;
        /// Describes the integer paired with the inner tuple.
        type Outer =
            cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Cons<Inner, cel_runtime::Nil>>;
        let value = parsed.read_compound::<Outer>(cell_id).unwrap();
        assert_eq!(value.1, 1);
        let (b, c) = (value.0.1.1, value.0.1.0.1);
        assert_eq!((b, c), (2.5, "x".to_string()));
    }

    /// Stores the complete result of an explicitly typed tuple output.
    #[test]
    fn parse_out_with_explicit_tuple_type_infers_and_stores_correctly() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell x: i32 = 3;
                    out pair: (i32, i32) := (x, x);
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let cell_id = *sheet.output_names.get("pair").unwrap();
        let value = sheet
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
        assert_eq!((a, b), (3, 3));
    }

    /// Reports an unsupported range initializer after syntax parsing succeeds.
    #[test]
    fn parse_cell_range_initializer_fails_cleanly_at_type_inference_not_grammar() {
        // `Range<i32>` isn't a registered adam-lang type — this must still fail today, but only
        // at `eval_segment_boxed`'s existing "cannot infer a type" check, proving the CEL-level
        // range parsing itself succeeded (rather than failing as "unexpected token" or similar
        // at the grammar level, which would indicate the entry-point swap didn't take effect).
        let result = parser().parse_str("sheet s { cell x = 1i32..5i32; }");
        assert!(result.is_err());
        let err = result.expect_err("expected Err");
        assert!(err.message().contains("cannot infer a type"));
    }

    /// Verifies unregistered out results identify the declaration and its name token.
    /// Retains output context and source span in a shape diagnostic.
    #[test]
    fn shape_error_out_preserves_context_and_span() {
        for expression in ["1i32..5i32", "(1i32, 1i32..5i32)"] {
            let source = format!("sheet s {{\nout result := {expression};\n}}");
            let err = parser().parse_str(&source).unwrap_err();
            assert!(err.message().contains("cannot infer a type"));
            assert_eq!(
                err.span().start(),
                proc_macro2::LineColumn { line: 2, column: 4 }
            );
            assert_eq!(
                err.span().end(),
                proc_macro2::LineColumn {
                    line: 2,
                    column: 10
                }
            );
            assert!(err.message().starts_with("out `result`:"));
        }
    }

    /// Verifies unregistered match results identify the expression and its opening token.
    /// Retains conditional-subject context and span in a shape diagnostic.
    #[test]
    fn shape_error_match_preserves_context_and_span() {
        for (expression, end_column) in [("1i32..5i32", 16), ("(1i32, 1i32..5i32)", 30)] {
            let source = format!("sheet s {{\nconditional {expression} {{ _ => {{}} }}\n}}");
            let err = parser().parse_str(&source).unwrap_err();
            assert!(err.message().contains("cannot infer a type"));
            assert_eq!(
                err.span().start(),
                proc_macro2::LineColumn {
                    line: 2,
                    column: 12
                }
            );
            assert_eq!(
                err.span().end(),
                proc_macro2::LineColumn {
                    line: 2,
                    column: end_column
                }
            );
            assert!(err.message().starts_with("match expression:"));
        }
    }

    /// Verifies unregistered binding results identify the output and its binding token.
    /// Retains binding-output context and span in a shape diagnostic.
    #[test]
    fn shape_error_output_preserves_context_and_span() {
        for (shape, expression) in [("i32", "1i32..5i32"), ("(i32, i32)", "(1i32, 1i32..5i32)")] {
            let source = format!(
                "sheet s {{\ncell result: {shape};\nrelationship {{\nresult := {expression};\n}}\n}}"
            );
            let err = parser().parse_str(&source).unwrap_err();
            assert!(err.message().contains("cannot infer a type"));
            assert_eq!(
                err.span().start(),
                proc_macro2::LineColumn { line: 4, column: 0 }
            );
            assert_eq!(
                err.span().end(),
                proc_macro2::LineColumn { line: 4, column: 6 }
            );
            assert!(err.message().starts_with("output `result`:"));
        }
    }

    /// Verifies unregistered initializer results identify the cell and its name token.
    /// Retains initializer context and source span in a shape diagnostic.
    #[test]
    fn shape_error_initializer_preserves_context_and_span() {
        for expression in ["1i32..5i32", "(1i32, 1i32..5i32)"] {
            let source = format!("sheet s {{\ncell result = {expression};\n}}");
            let err = parser().parse_str(&source).unwrap_err();
            assert!(err.message().contains("cannot infer a type"));
            assert_eq!(
                err.span().start(),
                proc_macro2::LineColumn { line: 2, column: 5 }
            );
            assert_eq!(
                err.span().end(),
                proc_macro2::LineColumn {
                    line: 2,
                    column: 11
                }
            );
            assert!(err.message().starts_with("cell `result`:"));
        }
    }

    /// Preserves source declaration labels and name spans for initializer errors.
    /// Retains source-cell context and span in a shape diagnostic.
    #[test]
    fn shape_error_source_preserves_context_and_span() {
        for (declaration, message) in [
            ("source result: i32 = true;", "type mismatch"),
            ("source result = 1i32..5i32;", "cannot infer a type"),
            ("source result = (1i32, 1i32..5i32);", "cannot infer a type"),
        ] {
            let source = format!("sheet s {{\n{declaration}\n}}");
            let err = parser().parse_str(&source).unwrap_err();
            assert!(err.message().contains(message));
            assert!(
                err.message().starts_with("source `result`:"),
                "{}",
                err.message()
            );
            assert_eq!(
                err.span().start(),
                proc_macro2::LineColumn { line: 2, column: 7 }
            );
            assert_eq!(
                err.span().end(),
                proc_macro2::LineColumn {
                    line: 2,
                    column: 13
                }
            );
        }
    }

    /// Verifies registered type mismatches retain the ordinary out and output prefixes.
    /// Retains declaration context for registered leaf mismatches.
    #[test]
    fn shape_error_registered_mismatches_preserve_context() {
        for (source, prefix) in [
            ("sheet s { out result: i32 := true; }", "out `result`:"),
            (
                "sheet s { cell result: i32; relationship { result := true; } }",
                "output `result`:",
            ),
        ] {
            let err = parser().parse_str(source).unwrap_err();
            assert!(err.message().contains("type mismatch"));
            assert!(err.message().starts_with(prefix));
        }
    }

    /// Attributes a dependency-cycle validation error to the governed cell.
    #[test]
    fn dependency_cycle_validation_error_spans_the_governed_cell() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The branch writes the match cell `mode`, so whole-sheet validation returns
        // DependencyCycle on line 2 (`cell mode`), the governed cell that starts the cycle.
        let source = "sheet s {\n    cell mode: i32 = 0;\n    cell other: i32 = 0;\n\n    conditional mode {\n        0i32 => {\n            relationship {\n                mode := other;\n                other := mode;\n            }\n        }\n    }\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(err.span().start().line, 2);
    }

    // NOTE on coverage: `add_conditional`'s other `InvalidConditional` case with a
    // `Relationship`-only site list -- the same relationship reused across two branches (or a
    // branch and the default) -- is not reachable through adam-lang's own grammar today.
    // `parse_branch_relationships` calls `parse_relationship_decl`, which always creates a
    // *fresh* `RelationshipId` on `add_relationship`; adam-lang has no syntax for a `relationship`
    // to be *named* and then referenced again from a second branch. That case is exercised
    // directly against `Sheet::add_conditional` in
    // `add_conditional_returns_invalid_conditional_for_duplicate_relationship_across_branches`
    // (adam-rs/src/sheet.rs); it stays untested at the adam-lang parser level until adam-lang
    // grows named/referenceable relationships (tracked as a follow-up, not blocking this task).
    /// Renders invalid conditional relationships and cells as related labels.
    #[test]
    fn conditional_invalid_relationship_and_cell_sites_render_as_secondary_labels() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The governed branch output `a` reaches match cell `p` through the unconditional
        // relationship, so the diagnostic should use `a` as primary and attach the
        // remaining cycle sites as secondary labels.
        let source = "sheet s {\n    cell a: i32 = 0;\n    cell b: i32 = 0;\n    cell p: i32 = 0;\n    relationship {\n        p := a;\n    }\n\n    conditional p {\n        0i32 => {\n            relationship {\n                a := b;\n            }\n        }\n    }\n}";
        let err = parser.parse_str(source).unwrap_err();
        let out =
            err.format_rustc_style(source, "t.adm2", 1, &annotate_snippets::Renderer::plain());
        assert!(out.contains("cell `p` is part of the cycle"), "{out}");
    }

    /// Attributes a duplicate output requirement to the repeated label.
    #[test]
    /// Locates duplicate labels at the repeated requirement, not the sheet opening.
    fn out_decl_duplicate_requirement_error_spans_the_duplicate_label() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The parser owns display-label validation, so it can identify the repeated label
        // itself rather than blaming runtime registration or the sheet's opening line.
        let source = "sheet s {\n    cell w: i32 = 1;\n    out a: i32 := w require {\n        @pos a > 0;\n        @pos a > 0;\n    };\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(err.span().start().line, 5);
        assert!(err.to_string().contains("duplicate requirement `pos`"));
    }
}
