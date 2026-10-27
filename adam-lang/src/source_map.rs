//! Owning source identities for diagnostics across module collections.
//!
//! A location retains its immutable file name and text even after its source map
//! is dropped. Native definitions use labels with no location.
//!
//! ```
//! use adam_lang::{SourceMap, SourceLabel, SourceLocation};
//! use cel_parser::SourceSpan;
//! let mut sources = SourceMap::new();
//! let source = sources.insert("example.adm2", "sheet example {}");
//! let label = SourceLabel {
//!     location: Some(SourceLocation { source, span: SourceSpan::new(1, 0, 1, 5) }),
//!     label: "sheet declaration".into(),
//! };
//! drop(sources);
//! assert_eq!(label.location.unwrap().source.file_name(), "example.adm2");
//! ```

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use cel_parser::SourceSpan;

/// Retains one immutable file name and its complete source text.
#[derive(Debug)]
struct SourceFile {
    file_name: String,
    text: String,
}

/// Identifies and owns one source independently of its collection's lifetime.
///
/// Equality and hashing use immutable ownership identity, not file contents.
/// Independent insertions remain distinct even with identical names and text.
/// See the [module example](self).
#[derive(Clone, Debug)]
pub struct SourceId {
    file: Rc<SourceFile>,
    index: usize,
}

impl PartialEq for SourceId {
    /// Compares immutable source ownership.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.file, &other.file)
    }
}

impl Eq for SourceId {}

impl Hash for SourceId {
    /// Hashes the same ownership identity used by equality.
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.file).hash(state);
    }
}

impl SourceId {
    /// Borrows the retained file name without consulting a collection or filesystem.
    ///
    /// # Examples
    ///
    /// See the [module example](self).
    pub fn file_name(&self) -> &str {
        &self.file.file_name
    }

    /// Borrows the complete retained source text.
    ///
    /// # Examples
    ///
    /// See the [module example](self).
    pub fn text(&self) -> &str {
        &self.file.text
    }
}

/// Collects immutable source owners in insertion order without filesystem IO.
///
/// Locations retain their own owners; dropping this collection does not invalidate
/// them. See the [module example](self).
#[derive(Debug, Default)]
pub struct SourceMap {
    sources: Vec<SourceId>,
}

impl SourceMap {
    /// Creates an empty source collection.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(adam_lang::SourceMap::new().sources().is_empty());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies a file name and source text into a new immutable source owner.
    ///
    /// Repeated insertions create distinct identities.
    /// - Complexity: O(f + t) time and space for file name length f and text length t,
    ///   plus amortized O(1) collection insertion.
    ///
    /// # Examples
    ///
    /// See the [module example](self).
    pub fn insert(&mut self, file_name: &str, text: &str) -> SourceId {
        let id = SourceId {
            file: Rc::new(SourceFile {
                file_name: file_name.into(),
                text: text.into(),
            }),
            index: self.sources.len(),
        };
        self.sources.push(id.clone());
        id
    }

    /// Returns the retained owner only if this collection contains `source`.
    ///
    /// Coincident indices in independent collections do not match.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut map = adam_lang::SourceMap::new();
    /// let id = map.insert("file.adm2", "");
    /// assert_eq!(map.get(&id), Some(&id));
    /// assert!(adam_lang::SourceMap::new().get(&id).is_none());
    /// ```
    pub fn get(&self, source: &SourceId) -> Option<&SourceId> {
        self.sources.get(source.index).filter(|id| *id == source)
    }

    /// Borrows source owners in insertion order.
    ///
    /// # Examples
    ///
    /// See [`Self::new`].
    pub fn sources(&self) -> &[SourceId] {
        &self.sources
    }
}

/// Retains a real source owner and its line/column span.
///
/// See the [module example](self).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    /// Owns the file name and text to which `span` refers.
    pub source: SourceId,
    /// Identifies the region within the retained source text.
    pub span: SourceSpan,
}

/// Labels a source region, or a source-free native definition without a location.
///
/// See the [module example](self).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLabel {
    /// Contains a real source location, or `None` for source-free definitions.
    pub location: Option<SourceLocation>,
    /// Describes the implicated definition or use.
    pub label: String,
}

/// Owns a module error message and all sources needed by its labels.
///
/// ```
/// let diagnostic = adam_lang::ModuleDiagnostic {
///     message: "invalid native use".into(),
///     labels: vec![adam_lang::SourceLabel { location: None, label: "native".into() }],
/// };
/// assert!(diagnostic.labels[0].location.is_none());
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleDiagnostic {
    /// Explains the failed module operation.
    pub message: String,
    /// Identifies implicated definitions and uses in diagnostic order.
    pub labels: Vec<SourceLabel>,
}

impl std::fmt::Display for ModuleDiagnostic {
    /// Formats the primary diagnostic message without rendering source labels.
    ///
    /// - Complexity: O(m) message bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModuleDiagnostic {}
