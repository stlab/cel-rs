//! Checks the supplied-module namespace phase, not executable-body compilation.

use adam_lang::{
    CompositionBuilder, ModuleDiagnostic, ModulePath, ModuleSet, SourceLabel, TypeRegistry,
};

/// Creates a validated module identifier.
fn path(name: &str) -> ModulePath {
    name.parse().unwrap()
}

/// Registers one syntactically valid source under its explicit path.
fn insert(modules: &mut ModuleSet, name: &str, source: &str) {
    modules
        .insert_source(path(name), &format!("{name}.adm2"), source)
        .unwrap();
}

/// Finds a diagnostic containing an expected semantic description.
fn error_containing<'a>(errors: &'a [ModuleDiagnostic], text: &str) -> &'a ModuleDiagnostic {
    errors
        .iter()
        .find(|error| error.message.contains(text))
        .unwrap_or_else(|| panic!("missing `{text}` in {errors:?}"))
}

/// Rejects empty, punctuated, raw, and incomplete paths.
#[test]
fn module_paths_validate_complete_identifiers() {
    for invalid in [
        "",
        "::a",
        "a::",
        "a:::b",
        "a b",
        "1a",
        "a-b",
        "a::_",
        "r#a",
        " a",
        "a ",
        "a:: b",
        "a//comment",
        "a/*comment*/",
    ] {
        assert!(invalid.parse::<ModulePath>().is_err(), "{invalid}");
    }
    let original = path("image::尺寸");
    let copy = original.clone();
    assert_eq!(copy, original);
    assert_eq!(original.to_string(), "image::尺寸");
    assert_ne!(original, path("image::other"));
}

/// Rejects reserved identifiers in every registered module path segment.
#[test]
fn reserved_module_path_names_are_rejected() {
    for name in ["sheet", "use", "as"] {
        for path in [
            name.to_owned(),
            format!("lib::{name}"),
            format!("{name}::lib"),
        ] {
            assert!(path.parse::<ModulePath>().is_err(), "{path}");
        }
    }
}

/// Rejects reserved native exports while retaining valid Unicode exports and aliases.
#[test]
fn reserved_native_export_names_are_rejected() {
    for name in ["sheet", "use", "as"] {
        let mut modules = ModuleSet::new();
        let template = CompositionBuilder::new(TypeRegistry::new())
            .finish()
            .unwrap();
        assert!(
            modules
                .insert_template(path("lib"), name, template)
                .is_err(),
            "{name}"
        );
        let template = CompositionBuilder::new(TypeRegistry::new())
            .finish()
            .unwrap();
        modules
            .insert_template(path("lib"), "尺寸", template)
            .unwrap();
        insert(
            &mut modules,
            "app",
            "use lib::尺寸 as 工作; sheet root { relationship 工作(); }",
        );
        modules.check_resolution().unwrap();
    }
}

/// Resolves forward components and aliases without exporting those aliases.
#[test]
fn aliases_and_forward_definitions_resolve() {
    let mut modules = ModuleSet::new();
    insert(
        &mut modules,
        "app",
        "use lib::wrapper as imported;
        sheet root { relationship imported(); }
        sheet first() { relationship second(); }
        sheet second() {}
        sheet another_root {}",
    );
    insert(&mut modules, "lib", "sheet wrapper() {}");
    modules.check_resolution().unwrap();
}

/// Resolves an imported wrapper in its defining module, not the caller's.
#[test]
fn imported_wrappers_keep_their_defining_namespace() {
    let mut modules = ModuleSet::new();
    insert(
        &mut modules,
        "lib",
        "sheet wrapper() { relationship helper(); } sheet helper() {}",
    );
    insert(
        &mut modules,
        "app",
        "use lib::wrapper;
        sheet helper {}
        sheet entry() { relationship wrapper(); }",
    );
    // The caller's helper is a root and would make the wrapper invalid if captured.
    modules.check_resolution().unwrap();
}

/// Limits qualified references to declared exports rather than imported aliases.
#[test]
fn import_aliases_are_not_reexports() {
    let mut modules = ModuleSet::new();
    insert(&mut modules, "lib", "sheet helper() {}");
    insert(&mut modules, "middle", "use lib::helper as local;");
    insert(&mut modules, "app", "use middle::local; sheet root {}");
    let errors = modules.check_resolution().unwrap_err();
    let error = error_containing(&errors, "does not export `local`");
    assert_eq!(
        error.labels[0]
            .location
            .as_ref()
            .unwrap()
            .source
            .file_name(),
        "app.adm2"
    );
}

/// Rejects missing modules, missing symbols, unresolved local uses and root invocations.
#[test]
fn missing_targets_and_root_uses_are_diagnostics() {
    for (source, message) in [
        ("use absent::helper; sheet root {}", "unknown module"),
        ("use lib::absent; sheet root {}", "does not export"),
        ("use helper; sheet root {}", "module and definition"),
        (
            "sheet root { relationship absent(); }",
            "unknown definition",
        ),
        (
            "sheet root { relationship lib::absent(); }",
            "does not export",
        ),
        (
            "sheet root { relationship lib::root(); }",
            "not a reusable component",
        ),
        (
            "sheet first {} sheet second { relationship first(); }",
            "not a reusable component",
        ),
    ] {
        let mut modules = ModuleSet::new();
        insert(&mut modules, "lib", "sheet helper() {} sheet root {}");
        insert(&mut modules, "app", source);
        let errors = modules.check_resolution().unwrap_err();
        assert!(!error_containing(&errors, message).labels.is_empty());
    }
}

/// Allows qualified component references without an explicit local import.
#[test]
fn explicit_paths_select_declared_components() {
    let mut modules = ModuleSet::new();
    insert(&mut modules, "outer::lib", "sheet helper() {}");
    insert(
        &mut modules,
        "app",
        "sheet root { relationship outer::lib::helper(); }",
    );
    modules.check_resolution().unwrap();
}

/// Diagnoses namespace collisions even when one import's target is invalid.
#[test]
fn duplicates_diagnose_instead_of_shadowing() {
    for source in [
        "sheet same() {} sheet same() {}",
        "use lib::helper as same; sheet same() {}",
        "use lib::helper as same; use lib::helper as same;",
        "use absent::helper as same; use lib::helper as same;",
    ] {
        let mut modules = ModuleSet::new();
        insert(&mut modules, "lib", "sheet helper() {}");
        insert(&mut modules, "app", source);
        let errors = modules.check_resolution().unwrap_err();
        let duplicate = error_containing(&errors, "duplicate");
        assert_eq!(duplicate.labels.len(), 2);
    }
}

/// Allows import-only cycles when authored component uses remain acyclic.
#[test]
fn import_only_cycles_are_valid() {
    let mut modules = ModuleSet::new();
    insert(&mut modules, "a", "use b::other; sheet local() {}");
    insert(&mut modules, "b", "use a::local; sheet other() {}");
    modules.check_resolution().unwrap();
}

/// Retains every use-site in an actual cycle independently of module ownership.
#[test]
fn actual_use_cycles_retain_their_use_site_chain() {
    let mut modules = ModuleSet::new();
    insert(
        &mut modules,
        "a",
        "use b::second;\nsheet first() { relationship second(); }",
    );
    insert(
        &mut modules,
        "b",
        "use a::first;\nsheet second() { relationship first(); }",
    );
    let errors = modules.check_resolution().unwrap_err();
    drop(modules);
    let cycle = error_containing(&errors, "cycle");
    assert_eq!(cycle.labels.len(), 2);
    let files = cycle
        .labels
        .iter()
        .map(|label| {
            let location = label.location.as_ref().unwrap();
            assert_eq!(location.span.start.line, 2);
            let range = location.span.to_byte_range(location.source.text());
            assert!(location.source.text()[range].starts_with("relationship"));
            location.source.file_name()
        })
        .collect::<Vec<_>>();
    assert!(files.contains(&"a.adm2"));
    assert!(files.contains(&"b.adm2"));
}

/// Checks unused definitions and conditional uses rather than only selected roots.
#[test]
fn all_authored_uses_participate_in_resolution() {
    for source in [
        "sheet unused() { relationship unused(); } sheet root {}",
        "sheet unused() { conditional true { true => { relationship missing(); } } }",
        "sheet unused() { conditional false { _ => { relationship missing(); } } }",
    ] {
        let mut modules = ModuleSet::new();
        insert(&mut modules, "app", source);
        assert!(modules.check_resolution().is_err(), "{source}");
    }
}

/// Combines native and parsed definitions without inventing native source spans.
#[test]
fn native_definitions_keep_supplied_labels() {
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let supplied = SourceLabel {
        location: None,
        label: "host helper".into(),
    };
    builder.set_label(supplied.clone());
    let native = builder.finish().unwrap();
    assert_eq!(native.definition_label(), &supplied);
    let mut modules = ModuleSet::new();
    modules
        .insert_template(path("lib"), "helper", native)
        .unwrap();
    insert(
        &mut modules,
        "lib",
        "sheet wrapper() { relationship helper(); }",
    );
    modules.check_resolution().unwrap();
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    builder.set_label(supplied);
    modules
        .insert_template(path("dupe"), "helper", builder.finish().unwrap())
        .unwrap();
    insert(&mut modules, "dupe", "sheet helper() {}");
    let errors = modules.check_resolution().unwrap_err();
    let duplicate = error_containing(&errors, "duplicate");
    assert!(
        duplicate
            .labels
            .iter()
            .any(|label| label.label == "host helper" && label.location.is_none())
    );
}

/// Keeps accepted declarations when a second source or native declaration is rejected.
#[test]
fn rejected_insertions_preserve_existing_declarations() {
    let mut modules = ModuleSet::new();
    insert(&mut modules, "lib", "sheet helper() {}");
    assert!(
        modules
            .insert_source(path("lib"), "second.adm2", "sheet other() {}")
            .is_err()
    );
    let native = CompositionBuilder::new(TypeRegistry::new())
        .finish()
        .unwrap();
    assert!(
        modules
            .insert_template(path("lib"), "helper", native)
            .is_err()
    );
    let native = CompositionBuilder::new(TypeRegistry::new())
        .finish()
        .unwrap();
    assert!(
        modules
            .insert_template(path("lib"), "not valid", native)
            .is_err()
    );
    insert(
        &mut modules,
        "app",
        "use lib::helper; sheet root { relationship helper(); }",
    );
    modules.check_resolution().unwrap();
}

/// Returns owning syntax-error labels and allows a corrected source insertion.
#[test]
fn malformed_insertions_retain_sources_without_reserving_modules() {
    for broken in ["sheet broken(", "sheet broken { cell x: ; }"] {
        let mut modules = ModuleSet::new();
        let error = modules
            .insert_source(path("app"), "broken.adm2", broken)
            .unwrap_err();
        insert(&mut modules, "app", "sheet corrected() {}");
        modules.check_resolution().unwrap();
        drop(modules);
        let owner = &error.labels[0].location.as_ref().unwrap().source;
        assert_eq!(owner.file_name(), "broken.adm2");
        assert_eq!(owner.text(), broken);
    }
}

/// Treats type/body checking as a later phase and never executes retained expressions.
#[test]
fn resolution_does_not_evaluate_or_typecheck_bodies() {
    let mut modules = ModuleSet::new();
    insert(
        &mut modules,
        "app",
        "sheet unused(x: Unknown) {
        cell private = not_registered();
        relationship { x := another_host_function(); }
    }",
    );
    modules.check_resolution().unwrap();
}

/// Checks empty collections and native definitions without invoking private initializers.
#[test]
fn resolution_borrows_native_definitions_without_execution() {
    use adam_lang::type_registry::TypeShape;
    use std::{any::TypeId, cell::Cell, rc::Rc};

    ModuleSet::default().check_resolution().unwrap();
    let calls = Rc::new(Cell::new(0usize));
    let capture = calls.clone();
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    builder
        .cell(
            "private",
            TypeShape::Named(TypeId::of::<i32>()),
            move || {
                let capture = capture.clone();
                let mut segment = cel_runtime::DynSegment::new::<()>();
                segment.op0(move || {
                    capture.set(capture.get().checked_add(1).unwrap());
                    7i32
                });
                Ok(segment)
            },
        )
        .unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_template(path("native"), "component", builder.finish().unwrap())
        .unwrap();
    insert(
        &mut modules,
        "app",
        "sheet root { relationship native::component(); }",
    );
    modules.check_resolution().unwrap();
    modules.check_resolution().unwrap();
    drop(modules);
    assert_eq!(calls.get(), 0);
}

/// Builds a source-free template retaining exactly the supplied definition label.
fn native_with_label(label: SourceLabel) -> adam_lang::TypedTemplate {
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    builder.set_label(label);
    builder.finish().unwrap()
}

/// Labels a real helper-name range with an independently owned native source.
fn located_native_label(file_name: &str, description: &str) -> SourceLabel {
    let source = adam_lang::SourceMap::new().insert(file_name, "sheet helper() {}");
    SourceLabel {
        location: Some(adam_lang::SourceLocation {
            source,
            span: cel_parser::SourceSpan::new(1, 6, 1, 12),
        }),
        label: description.into(),
    }
}

/// Preserves both genuine declaration owners when native insertion collides with parsed code.
#[test]
fn native_after_parsed_duplicates_retain_both_locations() {
    let mut modules = ModuleSet::new();
    insert(&mut modules, "lib", "sheet helper() {}");
    let incoming = native_with_label(located_native_label(
        "native-incoming.adm2",
        "incoming native",
    ));
    let error = modules
        .insert_template(path("lib"), "helper", incoming)
        .unwrap_err();
    drop(modules);

    assert!(error.message.contains("duplicate"));
    assert_eq!(error.labels.len(), 2);
    assert_eq!(error.labels[0].label, "incoming native");
    let files = error
        .labels
        .iter()
        .map(|label| {
            let location = label.location.as_ref().unwrap();
            let text = location.source.text();
            assert_eq!(&text[location.span.to_byte_range(text)], "helper");
            location.source.file_name()
        })
        .collect::<Vec<_>>();
    assert_eq!(files, ["native-incoming.adm2", "lib.adm2"]);
}

/// Preserves the span-free native site when a later parsed declaration collides.
#[test]
fn parsed_after_native_duplicates_retain_source_free_labels() {
    let mut modules = ModuleSet::new();
    modules
        .insert_template(
            path("lib"),
            "helper",
            native_with_label(SourceLabel {
                location: None,
                label: "existing native".into(),
            }),
        )
        .unwrap();
    insert(&mut modules, "lib", "sheet helper() {}");
    let errors = modules.check_resolution().unwrap_err();
    drop(modules);

    let error = error_containing(&errors, "duplicate");
    assert_eq!(error.labels.len(), 2);
    let parsed = error.labels[0].location.as_ref().unwrap();
    assert_eq!(parsed.source.file_name(), "lib.adm2");
    assert_eq!(parsed.source.text(), "sheet helper() {}");
    assert_eq!(error.labels[1].label, "existing native");
    assert!(error.labels[1].location.is_none());
}

/// Preserves incoming source-free and existing located native sites after collection drop.
#[test]
fn native_after_native_duplicates_retain_supplied_owners() {
    let mut modules = ModuleSet::new();
    modules
        .insert_template(
            path("lib"),
            "helper",
            native_with_label(located_native_label(
                "native-existing.adm2",
                "existing native",
            )),
        )
        .unwrap();
    let incoming = native_with_label(SourceLabel {
        location: None,
        label: "incoming native".into(),
    });
    let error = modules
        .insert_template(path("lib"), "helper", incoming)
        .unwrap_err();
    drop(modules);

    assert!(error.message.contains("duplicate"));
    assert_eq!(error.labels.len(), 2);
    assert_eq!(error.labels[0].label, "incoming native");
    assert!(error.labels[0].location.is_none());
    assert_eq!(error.labels[1].label, "existing native");
    let existing = error.labels[1].location.as_ref().unwrap();
    assert_eq!(existing.source.file_name(), "native-existing.adm2");
    assert_eq!(existing.source.text(), "sheet helper() {}");
    assert_eq!(
        &existing.source.text()[existing.span.to_byte_range(existing.source.text())],
        "helper"
    );
}

/// Handles deep dependency chains and shared descendants without recursive traversal.
#[test]
fn deep_compact_dependency_graphs_are_checked_iteratively() {
    let mut source = String::new();
    for index in 0..6000usize {
        source.push_str(&format!(
            "sheet d{index}() {{ relationship d{}(); relationship leaf(); }}\n",
            index + 1
        ));
    }
    source.push_str("sheet d6000() {} sheet leaf() {}");
    let mut modules = ModuleSet::new();
    insert(&mut modules, "deep", &source);
    modules.check_resolution().unwrap();
    drop(modules);
}
