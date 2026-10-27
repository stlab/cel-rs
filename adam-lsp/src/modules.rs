//! Adapts caller-owned module snapshots and open-document overlays to LSP diagnostics.
//!
//! The transport supplies `moduleSources: [{path, uri, text}]` at initialization and
//! in `workspace/didChangeConfiguration.settings`. Paths select registered modules;
//! URIs identify source owners. This module performs no filesystem IO or discovery.

use std::collections::{BTreeMap, HashMap, HashSet};

use adam_lang::{ModuleDiagnostic, ModulePath, ModuleSet, SourceLocation};
use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Position, Range, Uri,
};
use serde_json::Value;

use crate::diagnostics::{diagnostics_for_module, diagnostics_for_source};

/// Owns one validated wire record; paths and absolute URIs are unique in its snapshot.
struct ModuleSource {
    path: ModulePath,
    uri: Uri,
    text: String,
}

/// Owns the last valid explicit source snapshot and any visible refresh failure.
///
/// Failed refreshes block checking until a valid snapshot arrives; they never
/// switch a registered module to the source-only path.
#[derive(Default)]
pub(crate) struct ModuleSources {
    sources: Vec<ModuleSource>,
    error: Option<String>,
}

impl ModuleSources {
    /// Validates and replaces a complete wire snapshot, or retains a visible error.
    ///
    /// Missing `moduleSources` means an empty explicit mapping. A
    /// `moduleSourcesError` string reports a caller-side configuration/read failure.
    /// - Complexity: O(n + t) expected time and space for n records and t text bytes.
    pub(crate) fn configure(&mut self, options: &Value) {
        match parse_sources(options) {
            Ok(sources) => {
                self.sources = sources;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Rechecks the snapshot with URI overlays and every unregistered open source.
    ///
    /// Conservatively rechecks all registered definitions and republishes all
    /// registered/open files, including clean files that need old errors cleared.
    /// - Complexity: the underlying checkers and label projection, O(f log f)
    ///   output-map construction and O(fm) URI selection for f files and m modules.
    pub(crate) fn diagnostics(
        &self,
        documents: &HashMap<String, String>,
    ) -> BTreeMap<String, Vec<Diagnostic>> {
        let mut output: BTreeMap<String, Vec<Diagnostic>> = self
            .sources
            .iter()
            .map(|s| (s.uri.as_str().to_owned(), Vec::new()))
            .chain(documents.keys().map(|uri| (uri.clone(), Vec::new())))
            .collect();
        if let Some(error) = &self.error {
            for diagnostics in output.values_mut() {
                diagnostics.push(configuration_diagnostic(error));
            }
            return output;
        }
        let mut modules = ModuleSet::new();
        let mut errors = Vec::new();
        // Build one source-owner collection from the actual URI-selected revisions.
        for source in &self.sources {
            let text = documents.get(source.uri.as_str()).unwrap_or(&source.text);
            if let Err(error) =
                modules.insert_source(source.path.clone(), source.uri.as_str(), text)
            {
                errors.push(error);
            }
        }
        if errors.is_empty()
            && let Some(first) = self.sources.first()
        {
            errors = diagnostics_for_module(&first.path, &modules);
        }
        for error in &errors {
            append_module_diagnostic(&mut output, error);
        }
        for (uri, text) in documents {
            if !self.sources.iter().any(|source| source.uri.as_str() == uri) {
                output.insert(uri.clone(), diagnostics_for_source(text));
            }
        }
        output
    }
}

/// Parses the explicit wire schema and rejects malformed/ambiguous ownership.
///
/// - Complexity: O(n + t) expected time and space for n records and t text bytes.
/// # Errors
/// Returns a visible configuration/read error rather than an empty success.
fn parse_sources(options: &Value) -> Result<Vec<ModuleSource>, String> {
    if !options.is_null() && !options.is_object() {
        return Err("module source options must be an object".into());
    }
    if let Some(error) = options.get("moduleSourcesError") {
        return Err(format!("module source refresh failed: {error}"));
    }
    let Some(records) = options.get("moduleSources") else {
        return Ok(Vec::new());
    };
    let records = records
        .as_array()
        .ok_or("moduleSources must be an array of {path, uri, text}")?;
    let mut paths = HashSet::new();
    let mut uris = HashSet::new();
    records
        .iter()
        .map(|record| {
            let field = |name| {
                record
                    .get(name)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("moduleSources record requires string `{name}`"))
            };
            let path: ModulePath = field("path")?
                .parse()
                .map_err(|error| format!("invalid module path: {error}"))?;
            let uri: Uri = field("uri")?
                .parse()
                .map_err(|error| format!("invalid module URI: {error}"))?;
            if uri.scheme().is_none() {
                return Err("module URI must be absolute".into());
            }
            if !paths.insert(path.clone()) || !uris.insert(uri.as_str().to_owned()) {
                return Err("moduleSources paths and URIs must each be unique".into());
            }
            Ok(ModuleSource {
                path,
                uri,
                text: field("text")?.to_owned(),
            })
        })
        .collect()
}

/// Creates an error-severity configuration diagnostic without an invented source span.
fn configuration_diagnostic(message: &str) -> Diagnostic {
    Diagnostic {
        range: Range::default(),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("adam-lang configuration".into()),
        message: message.into(),
        ..Default::default()
    }
}

/// Publishes one error at each owning file with all real labels as related information.
///
/// Source-free labels remain in the message; unlocated errors reach every supplied file.
/// - Complexity: O(fl + t + p) time and O(l + p) space for f files, l labels,
///   scanned owning text t and published output p.
fn append_module_diagnostic(
    output: &mut BTreeMap<String, Vec<Diagnostic>>,
    error: &ModuleDiagnostic,
) {
    let locations: Vec<_> = error
        .labels
        .iter()
        .filter_map(|label| {
            let location = label.location.as_ref()?;
            let uri = location.source.file_name().parse::<Uri>().ok()?;
            Some((
                label,
                Location {
                    uri,
                    range: location_range(location),
                },
            ))
        })
        .collect();
    let mut message = error.message.clone();
    for label in &error.labels {
        if label.location.is_none() {
            message.push_str(&format!("\n{}", label.label));
        }
    }
    for (uri, diagnostics) in output.iter_mut() {
        let primary = locations
            .iter()
            .find(|(_, location)| location.uri.as_str() == uri);
        if primary.is_none() && !locations.is_empty() {
            continue;
        }
        diagnostics.push(Diagnostic {
            range: primary.map(|(_, l)| l.range).unwrap_or_default(),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("adam-lang".into()),
            message: message.clone(),
            related_information: Some(
                locations
                    .iter()
                    .map(|(label, location)| DiagnosticRelatedInformation {
                        location: location.clone(),
                        message: label.label.clone(),
                    })
                    .collect(),
            ),
            ..Default::default()
        });
    }
}

/// Converts character columns against the actual retained text into UTF-16 LSP positions.
///
/// - Complexity: O(n) time for the source prefix ending at the span.
fn location_range(location: &SourceLocation) -> Range {
    let position = |line: usize, column: usize| {
        let character = location
            .source
            .text()
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
        position(location.span.start.line, location.span.start.column),
        position(location.span.end.line, location.span.end.column),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_lang::{SourceLabel, SourceMap};
    use cel_parser::SourceSpan;
    use serde_json::json;

    /// Supplies a real registered definition and importer for snapshot contracts.
    fn snapshot() -> Value {
        json!({"moduleSources":[
            {"path":"lib","uri":"file:///lib.adm2","text":"sheet pair(a: f64) {}"},
            {"path":"app","uri":"file:///app.adm2","text":"use lib::pair; sheet app { cell x: f64; relationship pair(a: x); }"}
        ]})
    }

    /// Malformed wire data stays an observable runtime error, including duplicate owners.
    #[test]
    fn invalid_wire_never_becomes_source_only_success() {
        for options in [
            json!(42),
            json!({"moduleSources":null}),
            json!({"moduleSources":[{}]}),
            json!({"moduleSources":[{"path":"bad-path","uri":"file:///x","text":""}]}),
            json!({"moduleSources":[{"path":"ok","uri":"relative.adm2","text":""}]}),
            json!({"moduleSources":[{"path":"ok","uri":"file:///x","text":1}]}),
            json!({"moduleSources":[{"path":"ok","uri":"file:///x","text":""},{"path":"ok","uri":"file:///y","text":""}]}),
            json!({"moduleSources":[{"path":"ok","uri":"file:///x","text":""},{"path":"other","uri":"file:///x","text":""}]}),
            json!({"moduleSourcesError":"read denied"}),
        ] {
            let mut registry = ModuleSources::default();
            registry.configure(&options);
            let documents = HashMap::from([("file:///open.adm2".into(), "sheet good {}".into())]);
            let output = registry.diagnostics(&documents);
            assert!(!output["file:///open.adm2"].is_empty(), "{options}");
            assert!(
                output["file:///open.adm2"][0]
                    .source
                    .as_ref()
                    .unwrap()
                    .contains("configuration")
            );
        }
    }

    /// Closing a definition restores its supplied revision; refresh errors block and recover.
    #[test]
    fn source_refresh_and_close_preserve_explicit_ownership() {
        let mut registry = ModuleSources::default();
        registry.configure(&snapshot());
        let definition = "file:///lib.adm2".to_owned();
        let mut documents = HashMap::from([(definition.clone(), "sheet pair(a: i32) {}".into())]);
        let errors = registry.diagnostics(&documents);
        assert!(!errors["file:///app.adm2"].is_empty());
        registry.configure(&snapshot());
        assert!(!registry.diagnostics(&documents)["file:///app.adm2"].is_empty());
        documents.remove(&definition);
        assert!(registry.diagnostics(&documents).values().all(Vec::is_empty));
        registry.configure(&json!({"moduleSourcesError":"deleted"}));
        assert!(
            registry
                .diagnostics(&documents)
                .values()
                .all(|d| d[0].message.contains("deleted"))
        );
        registry.configure(&snapshot());
        assert!(registry.diagnostics(&documents).values().all(Vec::is_empty));
        registry.configure(&json!({"moduleSources":[]}));
        assert!(registry.diagnostics(&documents).is_empty());
    }

    /// Parse failures retain the actual overlay owner rather than substituting empty text.
    #[test]
    fn syntax_errors_and_unregistered_sources_keep_separate_checking_paths() {
        let mut registry = ModuleSources::default();
        registry.configure(&snapshot());
        let documents = HashMap::from([
            ("file:///lib.adm2".into(), "use".into()),
            (
                "file:///standalone.adm2".into(),
                "sheet s { cell x: i32 = true; }".into(),
            ),
        ]);
        let errors = registry.diagnostics(&documents);
        assert!(!errors["file:///lib.adm2"].is_empty());
        assert!(!errors["file:///standalone.adm2"].is_empty());
        assert!(
            errors["file:///lib.adm2"]
                .iter()
                .flat_map(|d| d.related_information.iter().flatten())
                .all(|r| r.location.uri.as_str() == "file:///lib.adm2")
        );
    }

    /// Source-free native context is not discarded or assigned a fabricated location.
    #[test]
    fn native_labels_remain_visible_without_locations() {
        let mut output = BTreeMap::from([("file:///app.adm2".into(), Vec::new())]);
        append_module_diagnostic(
            &mut output,
            &ModuleDiagnostic {
                message: "invalid native use".into(),
                labels: vec![SourceLabel {
                    location: None,
                    label: "host definition".into(),
                }],
                cause: None,
            },
        );
        assert!(
            output["file:///app.adm2"][0]
                .message
                .contains("host definition")
        );
        assert!(
            output["file:///app.adm2"][0]
                .related_information
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }

    /// Cross-file range conversion uses the owning text, including non-BMP characters.
    #[test]
    fn ranges_count_utf16_in_the_owning_source() {
        let mut sources = SourceMap::new();
        let location = SourceLocation {
            source: sources.insert("file:///unicode.adm2", "old\n😀 é value"),
            span: SourceSpan::new(2, 4, 2, 9),
        };
        assert_eq!(
            location_range(&location),
            Range::new(Position::new(1, 5), Position::new(1, 10))
        );
    }
}
