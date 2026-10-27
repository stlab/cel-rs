//! Collects supplied modules and checks definition lookup without filesystem access.
//!
//! ```
//! use adam_lang::{ModulePath, ModuleSet};
//! let path: ModulePath = "constraints".parse()?;
//! let mut modules = ModuleSet::new();
//! modules.insert_source(path.clone(), "constraints.adm2", "sheet empty() {}")?;
//! modules.check_resolution().map_err(|errors| errors.into_iter().next().unwrap())?;
//! # Ok::<(), adam_lang::ModuleDiagnostic>(())
//! ```

mod resolve;

use std::{collections::HashMap, fmt, str::FromStr};

use cel_parser::SourceSpan;

use crate::{
    AdamAstParser, ModuleDiagnostic, SourceId, SourceLabel, SourceLocation, SourceMap,
    TypedTemplate, ast,
};

/// Identifies a supplied module by nonempty, `::`-separated identifiers.
///
/// Whitespace, raw identifiers, and `_` alone are not path segments.
/// See the [module example](self).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModulePath(Vec<String>);

impl FromStr for ModulePath {
    type Err = ModuleDiagnostic;

    /// Validates and retains every identifier segment.
    ///
    /// # Errors
    /// Rejects empty paths or invalid segments.
    /// - Complexity: O(n) path bytes.
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let segments: Vec<_> = path.split("::").collect();
        if !segments.iter().all(|segment| valid_identifier(segment)) {
            return Err(crate::composition::diagnostic("invalid module path"));
        }
        Ok(Self(segments.into_iter().map(str::to_owned).collect()))
    }
}

impl fmt::Display for ModulePath {
    /// Writes the validated path with `::` separators.
    ///
    /// - Complexity: O(n) path bytes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, segment) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str("::")?;
            }
            formatter.write_str(segment)?;
        }
        Ok(())
    }
}

/// Checks one complete ordinary identifier without accepting surrounding whitespace.
///
/// - Complexity: O(n) identifier bytes.
fn valid_identifier(name: &str) -> bool {
    if name.is_empty() || name == "_" || name.starts_with("r#") {
        return false;
    }
    let Ok(stream) = name.parse::<proc_macro2::TokenStream>() else {
        return false;
    };
    let mut tokens = stream.into_iter();
    matches!(tokens.next(), Some(proc_macro2::TokenTree::Ident(ident)) if ident == name)
        && tokens.next().is_none()
}

/// Owns parsed and native declarations under explicit module paths.
///
/// A module has at most one source file and may also contain native definitions.
/// Insertion parses syntax only. [`Self::check_resolution`] checks names and cycles,
/// not types, actual cells, executable bodies or graph solvability.
/// See the [module example](self).
#[derive(Default)]
pub struct ModuleSet {
    entries: Vec<ModuleEntry>,
    indices: HashMap<ModulePath, usize>,
    sources: SourceMap,
}

/// Owns a module's declared exports and explicit imports in authored order.
struct ModuleEntry {
    path: ModulePath,
    source: Option<SourceId>,
    imports: Vec<ast::UseDecl>,
    definitions: Vec<Definition>,
}

/// Owns one source or native definition without expanding inclusion edges.
struct Definition {
    name: String,
    body: DefinitionBody,
}

/// Distinguishes retained authored bodies from already checked native recipes.
enum DefinitionBody {
    Parsed {
        sheet: Box<ast::Sheet>,
        label: SourceLabel,
    },
    Native(TypedTemplate),
}

impl Definition {
    /// Borrows the parsed declaration's or programmatic template's genuine label.
    fn label(&self) -> &SourceLabel {
        match &self.body {
            DefinitionBody::Parsed { label, .. } => label,
            DefinitionBody::Native(template) => template.definition_label(),
        }
    }
}

impl ModuleSet {
    /// Creates an empty supplied module collection.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses one source file and adds its declarations without executing expressions.
    ///
    /// A rejected insertion leaves existing declarations unchanged.
    /// # Errors
    /// Rejects malformed syntax or a second source file for the same module.
    /// Diagnostics own their source even after this collection is dropped.
    /// - Complexity: O(n) source and AST size, excluding embedded CEL parsing.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn insert_source(
        &mut self,
        path: ModulePath,
        file_name: &str,
        source: &str,
    ) -> Result<(), ModuleDiagnostic> {
        if let Some(index) = self.indices.get(&path)
            && self.entries[*index].source.is_some()
        {
            return Err(crate::composition::diagnostic(
                "module already has a source file",
            ));
        }
        let owner = self.sources.insert(file_name, source);
        let parsed = AdamAstParser::new()
            .parse_module_str(source)
            .map_err(|error| parse_error(&owner, &error))?;
        if let Some(error) = parsed.errors.first() {
            return Err(parse_error(&owner, error));
        }
        let entry = self.entry(path);
        entry.source = Some(owner.clone());
        for item in parsed.items {
            match item {
                ast::ModuleItem::Use(import) => entry.imports.push(import),
                ast::ModuleItem::Sheet(sheet) => entry.definitions.push(Definition {
                    name: sheet.name.clone(),
                    body: DefinitionBody::Parsed {
                        label: label(&owner, sheet.name_span, &sheet.name),
                        sheet: Box::new(sheet),
                    },
                }),
                ast::ModuleItem::Error(_) => unreachable!("recovered errors rejected above"),
            }
        }
        Ok(())
    }

    /// Adds an immutable source-free definition to a supplied module.
    ///
    /// Native and parsed definitions share the module's export namespace.
    /// # Errors
    /// Rejects invalid identifiers or an already declared local definition.
    /// - Complexity: O(d + b) existing local definitions and compared name bytes.
    ///
    /// # Examples
    /// ```
    /// let mut modules = adam_lang::ModuleSet::new();
    /// let native = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new()).finish()?;
    /// modules.insert_template("native".parse()?, "empty", native)?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn insert_template(
        &mut self,
        path: ModulePath,
        name: &str,
        template: TypedTemplate,
    ) -> Result<(), ModuleDiagnostic> {
        if !valid_identifier(name) {
            return Err(crate::composition::diagnostic("invalid definition name"));
        }
        let entry = self.entry(path);
        if entry
            .definitions
            .iter()
            .any(|definition| definition.name == name)
        {
            return Err(crate::composition::diagnostic("duplicate local definition"));
        }
        entry.definitions.push(Definition {
            name: name.into(),
            body: DefinitionBody::Native(template),
        });
        Ok(())
    }

    /// Checks definition names, explicit imports and actual component-use cycles.
    ///
    /// Resolves imported bodies in their defining module. Aliases do not re-export.
    /// Checks every authored use, including unused definitions and conditional
    /// branches. Import-only cycles are valid. No expression executes.
    /// This phase does not check types, actual-cell bindings, executable bodies,
    /// branch eligibility or whole-graph validity.
    ///
    /// # Errors
    /// Reports duplicate names, missing modules/exports, uses of roots and actual-use
    /// cycles. Returned labels retain sources independently of this collection.
    /// - Complexity: O(n + p) time for file/body/branch declarations n and
    ///   qualified-name bytes p; O(d + i + u + p) additional space for definitions,
    ///   imports, uses and names.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn check_resolution(&self) -> Result<(), Vec<ModuleDiagnostic>> {
        resolve::resolve(self).map(|_| ())
    }

    /// Returns or creates the unique module entry for a validated path.
    ///
    /// - Complexity: Amortized O(p) path bytes.
    fn entry(&mut self, path: ModulePath) -> &mut ModuleEntry {
        let index = *self.indices.entry(path.clone()).or_insert_with(|| {
            let index = self.entries.len();
            self.entries.push(ModuleEntry {
                path,
                source: None,
                imports: Vec::new(),
                definitions: Vec::new(),
            });
            index
        });
        &mut self.entries[index]
    }
}

/// Labels an authored span while retaining its source owner.
///
/// - Complexity: O(n) label bytes.
fn label(source: &SourceId, span: ast::ExprSpan, text: &str) -> SourceLabel {
    SourceLabel {
        location: Some(SourceLocation {
            source: source.clone(),
            span: SourceSpan::from_proc_macro2_range(span.start, span.end),
        }),
        label: text.into(),
    }
}

/// Produces an owning diagnostic for a parser or CEL compiler span.
///
/// - Complexity: O(n) message bytes.
fn source_error(
    source: &SourceId,
    span: SourceSpan,
    message: impl fmt::Display,
) -> ModuleDiagnostic {
    let message = message.to_string();
    ModuleDiagnostic {
        labels: vec![SourceLabel {
            location: Some(SourceLocation {
                source: source.clone(),
                span,
            }),
            label: message.clone(),
        }],
        message,
    }
}

/// Retains the parser's primary range and every secondary source label.
///
/// - Complexity: O(n) message and label bytes.
fn parse_error(source: &SourceId, error: &crate::ParseError) -> ModuleDiagnostic {
    let span = SourceSpan::from_proc_macro2_range(
        error.span(),
        error.end_span().unwrap_or_else(|| error.span()),
    );
    let mut diagnostic = source_error(source, span, error.message());
    diagnostic
        .labels
        .extend(error.secondary().iter().map(|secondary| SourceLabel {
            location: Some(SourceLocation {
                source: source.clone(),
                span: secondary.span,
            }),
            label: secondary.label.clone(),
        }));
    diagnostic
}
