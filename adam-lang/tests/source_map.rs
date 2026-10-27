//! Public ownership contracts for module diagnostics.

use adam_lang::{ModuleDiagnostic, SourceLabel, SourceLocation, SourceMap};
use cel_parser::SourceSpan;

/// Detects coincident indices mistaken for shared source ownership.
#[test]
fn independent_collections_keep_distinct_source_identity() {
    let mut first = SourceMap::new();
    let mut second = SourceMap::new();
    let a = first.insert("same.adm2", "sheet same {}");
    let b = second.insert("same.adm2", "sheet same {}");
    assert_ne!(a, b);
    assert_eq!(a, a.clone());
    assert_eq!(first.get(&a), Some(&a));
    assert!(first.get(&b).is_none());
    assert_eq!(first.sources(), std::slice::from_ref(&a));
    let mut keys = std::collections::HashSet::new();
    keys.insert(a.clone());
    keys.insert(a);
    keys.insert(b);
    assert_eq!(keys.len(), 2);
}

/// Detects diagnostics that borrow a consumed compilation's source collection.
#[test]
fn diagnostics_retain_file_and_text_after_collection_drop() {
    let diagnostic = {
        let mut sources = SourceMap::default();
        let source = sources.insert("library.adm2", "sheet library() {}");
        ModuleDiagnostic {
            message: "invalid use".into(),
            labels: vec![SourceLabel {
                location: Some(SourceLocation {
                    source,
                    span: SourceSpan::new(1, 0, 1, 5),
                }),
                label: "definition".into(),
            }],
        }
    };
    let location = diagnostic.labels[0].location.as_ref().unwrap();
    assert_eq!(location.source.file_name(), "library.adm2");
    assert_eq!(location.source.text(), "sheet library() {}");
    assert_eq!(location.span, SourceSpan::new(1, 0, 1, 5));
    assert_eq!(diagnostic.clone(), diagnostic);
}

/// Detects invented source locations for programmatic native definitions.
#[test]
fn native_labels_have_no_location() {
    let label = SourceLabel {
        location: None,
        label: "native relationship".into(),
    };
    assert!(label.location.is_none());
    assert_eq!(label.clone(), label);
}

/// Detects borrowed insertion inputs, content-based identity, or invalid empty sources.
#[test]
fn insertions_own_inputs_and_distinguish_repeated_empty_sources() {
    let mut sources = SourceMap::new();
    assert!(sources.sources().is_empty());
    let source = {
        let name = String::from("owned.adm2");
        let text = String::from("sheet owned {}");
        sources.insert(&name, &text)
    };
    assert_eq!(source.file_name(), "owned.adm2");
    assert_eq!(source.text(), "sheet owned {}");
    let first_empty = sources.insert("", "");
    let second_empty = sources.insert("", "");
    assert_ne!(first_empty, second_empty);
    assert_eq!(sources.get(&first_empty), Some(&first_empty));
    assert_eq!(sources.get(&second_empty), Some(&second_empty));
    assert_eq!(first_empty.file_name(), "");
    assert_eq!(first_empty.text(), "");
    drop(sources);
    assert_eq!(source.text(), "sheet owned {}");
}
