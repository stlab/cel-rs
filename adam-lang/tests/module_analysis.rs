//! Exercises nonexecuting module checking through the public source-owning interface.

use adam_lang::{ModulePath, ModuleSet, TypeRegistry, check_module};
use cel_parser::OpLookup;
use std::cell::Cell;

/// Checks a registered file without creating a live sheet.
fn check(source: &str) -> Vec<adam_lang::ModuleDiagnostic> {
    let path: ModulePath = "test".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(path.clone(), "actual.adm2", source)
        .unwrap();
    check_module(&path, &modules, &TypeRegistry::new(), &OpLookup::new())
}

/// Rejects invalid bodies even when no root uses them or source binding would prune them.
#[test]
fn module_analysis_checks_all_directions_and_declaration_order() {
    for source in [
        "sheet bad(a: i32) { relationship { a := true; } }",
        "sheet bad() { out x := later; cell later: i32; }",
        "sheet bad(a: i32) { cell x: i32 = a; }",
        "sheet bad(a: Missing) {}",
        "sheet bad(a: i32) { cell a: i32; }",
        "sheet bad(a: i32) { relationship { (a, a) := (1, 2); } }",
        "sheet bad(a: i32) { relationship {} }",
        "sheet bad(a: i32) { relationship { a := a; a := a; } }",
        "sheet bad() { cell a: i32 filter 2; }",
        "sheet bad() { cell a: i32 require { a + 1; }; }",
        "sheet bad() { cell a: i32; conditional a { true => {} } }",
    ] {
        assert!(!check(source).is_empty(), "{source}");
    }
    assert!(check("sheet good(a: i32, b: i32) { relationship { a := b; b := a; } }").is_empty());
}

/// Requires complete, distinct, shape-exact named arguments to already declared non-out cells.
#[test]
fn module_analysis_checks_named_bindings_and_full_shapes() {
    let prefix = "sheet pair(a: [i32], b: [i32]) {} sheet root { cell x: [i32]; cell y: [i32]; cell z: [f64]; ";
    for invocation in [
        "relationship pair(a: x);",
        "relationship pair(a: x, a: y);",
        "relationship pair(a: x, b: x);",
        "relationship pair(a: x, b: z);",
        "relationship pair(a: x, c: y);",
        "relationship pair(a: x, b: missing);",
        "out o := x; relationship pair(a: x, b: o);",
    ] {
        assert!(
            !check(&format!("{prefix}{invocation}}}")).is_empty(),
            "{invocation}"
        );
    }
    assert!(check(&format!("{prefix}relationship pair(b: y, a: x);}}")).is_empty());
}

/// Classifies branch uses transitively and specializes entire methods in inactive branches too.
#[test]
fn module_analysis_checks_branch_eligibility_and_source_specialization() {
    let ordinary = "sheet leaf(a: i32, b: i32) { relationship { a := b; b := a; } }";
    assert!(
        check(&format!(
            "{ordinary} sheet root {{ source x: i32; cell y: i32; relationship leaf(a: x, b: y); }}"
        ))
        .is_empty()
    );
    assert!(!check(&format!("{ordinary} sheet root {{ source x: i32; source y: i32; conditional false {{ true => {{ relationship leaf(a: x, b: y); }} }} }}")).is_empty());
    assert!(!check("sheet leaf(a: i32) { cell private: i32; } sheet middle(a: i32) { relationship leaf(a: a); } sheet root { cell x: i32; conditional false { true => { relationship middle(a: x); } } }").is_empty());
    assert!(check(&format!("{ordinary} sheet middle(a: i32, b: i32) {{ relationship leaf(a: a, b: b); }} sheet root {{ cell x: i32; cell y: i32; conditional false {{ true => {{ relationship middle(a: x, b: y); }} }} }}")).is_empty());
    assert!(!check("sheet triple(a: i32, b: i32, c: i32) { relationship { (a,b) := (c,c); c := a + b; } } sheet root { source x: i32; cell y: i32; source z: i32; relationship triple(a: x,b: y,c: z); }").is_empty());
}

/// Checks every body form while preserving ordinary initializer restrictions.
#[test]
fn module_analysis_accepts_complete_bodies() {
    let errors = check(
        "sheet all(a: i32, b: i32) {
        cell private: i32 filter _ + a require { private >= 0; };
        source limit: i32 = 10 filter 0..=20;
        out result: i32 := a + private require { result >= 0; };
        relationship { a := b; b := a; }
        conditional limit { 10 => { relationship { a := b; b := a; } } _ => {} }
    }",
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!check("sheet bad(a: i32) { source x: i32 = a; }").is_empty());
    assert!(check("sheet good() { cell a: (i32, [f64]); source b = 2; out c := a; }").is_empty());
}

/// Uses genuine file ownership for diagnostics and rejects an unknown requested module.
#[test]
fn module_analysis_preserves_source_ownership() {
    let path: ModulePath = "test".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "real.adm2",
            "sheet bad(a: i32) { relationship { a := true; } }",
        )
        .unwrap();
    let diagnostics = check_module(&path, &modules, &TypeRegistry::new(), &OpLookup::new());
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .flat_map(|d| &d.labels)
            .any(|l| l.location.is_some())
    );
    assert!(
        !check_module(
            &"missing".parse().unwrap(),
            &modules,
            &TypeRegistry::new(),
            &OpLookup::new()
        )
        .is_empty()
    );
    drop(modules);
    let location = diagnostics[0].labels[0].location.as_ref().unwrap();
    assert_eq!(location.source.file_name(), "real.adm2");
    assert_eq!(
        location.source.text(),
        "sheet bad(a: i32) { relationship { a := true; } }"
    );
    assert_eq!(location.span.start.line, 1);
}

/// Checks arbitrary registered host descriptors without executing any host callback.
#[test]
fn module_analysis_never_executes_host_code() {
    /// Supplies a counted host default with ordinary cell capabilities.
    #[derive(Clone, Debug, PartialEq)]
    struct Host(u8);
    thread_local! {
        static DEFAULTS: Cell<usize> = const { Cell::new(0) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }
    impl Default for Host {
        /// Counts actual value construction.
        fn default() -> Self {
            DEFAULTS.with(|count| count.set(count.get() + 1));
            Self(0)
        }
    }
    let mut types = TypeRegistry::new();
    types.register::<Host>("Host");
    let mut lookup = OpLookup::new();
    lookup.push_scope(move |name, segment, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        match name {
            "host" => {
                segment.op0(|| {
                    CALLS.with(|count| count.set(count.get() + 1));
                    Host(0)
                });
            }
            "probe" => {
                segment.op0(|| {
                    CALLS.with(|count| count.set(count.get() + 1));
                    1i32
                });
            }
            _ => return Ok(false),
        }
        Ok(true)
    });
    for body in [
        "relationship host_copy(a: left, b: right);",
        "cell default: Host; cell initial: Host = host; out result := host;",
        "cell a: i32 = probe filter _ + probe require { a > probe; }; conditional probe { 1 => { relationship { a := probe; } } }",
        "cell a: Host; relationship { a := true; }",
    ] {
        let path: ModulePath = "host".parse().unwrap();
        let mut modules = ModuleSet::new();
        let body = if body.starts_with("relationship host_copy") {
            format!(
                "sheet host_copy(a: (Host, [Host]), b: (Host, [Host])) {{ relationship {{ a := b; b := a; }} }} sheet s() {{ cell left: (Host, [Host]); cell right: (Host, [Host]); {body} }}"
            )
        } else {
            format!("sheet s() {{ {body} }}")
        };
        modules
            .insert_source(path.clone(), "host.adm2", &body)
            .unwrap();
        let errors = check_module(&path, &modules, &types, &lookup);
        if body.contains("a := true") {
            assert!(!errors.is_empty());
        } else {
            assert!(errors.is_empty(), "{errors:?}");
        }
        CALLS.with(|count| assert_eq!(count.get(), 0));
        DEFAULTS.with(|count| assert_eq!(count.get(), 0));
    }
    let expression = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
        .parse_str_ast("probe")
        .unwrap();
    let mut segment = cel_parser::compile_expr(
        &expression,
        &lookup,
        &types.cel_type_resolver(),
        &mut |_, _, _| Ok(false),
    )
    .unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 1);
    CALLS.with(|count| assert_eq!(count.get(), 1));
    let _ = Host::default();
    DEFAULTS.with(|count| assert_eq!(count.get(), 1));
    types.register_no_default::<Host>("Host");
    for (body, valid) in [
        ("cell value: Host;", false),
        ("cell values: [Host];", true),
        ("source value: Host = host;", true),
    ] {
        let path: ModulePath = "defaults".parse().unwrap();
        let mut modules = ModuleSet::new();
        modules
            .insert_source(
                path.clone(),
                "defaults.adm2",
                &format!("sheet s() {{ {body} }}"),
            )
            .unwrap();
        let errors = check_module(&path, &modules, &types, &lookup);
        assert_eq!(errors.is_empty(), valid, "{errors:?}");
        CALLS.with(|count| assert_eq!(count.get(), 1));
        DEFAULTS.with(|count| assert_eq!(count.get(), 1));
    }
}

/// Inspects native signatures and reuses native source pruning without invoking factories.
#[test]
fn module_analysis_checks_native_dependencies() {
    let mut builder = adam_lang::CompositionBuilder::new(TypeRegistry::new());
    let shape = adam_lang::type_registry::TypeShape::Named(std::any::TypeId::of::<i32>());
    let a = builder.parameter("a", shape.clone()).unwrap();
    let b = builder.parameter("b", shape).unwrap();
    let first = builder
        .method(std::slice::from_ref(&b), std::slice::from_ref(&a), || {
            let mut segment = cel_runtime::DynSegment::new::<()>();
            segment.push_arg::<i32>(0);
            Ok(segment)
        })
        .unwrap();
    let second = builder
        .method(&[a], &[b], || {
            let mut segment = cel_runtime::DynSegment::new::<()>();
            segment.push_arg::<i32>(0);
            Ok(segment)
        })
        .unwrap();
    builder.relationship(vec![first, second]).unwrap();
    let template = builder.finish().unwrap();
    assert_eq!(
        template
            .parameters()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let mut modules = ModuleSet::new();
    modules
        .insert_template("native".parse().unwrap(), "copy", template)
        .unwrap();
    let path: ModulePath = "caller".parse().unwrap();
    modules.insert_source(path.clone(), "caller.adm2",
        "use native::copy; sheet root { source x: i32; source y: i32; conditional false { true => { relationship copy(a: x, b: y); } } }").unwrap();
    let errors = check_module(&path, &modules, &TypeRegistry::new(), &OpLookup::new());
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .flat_map(|error| &error.labels)
            .any(|label| label.location.is_none())
    );
}

/// Does not hide invalid directions behind source arguments or unused foreign-module definitions.
#[test]
fn module_analysis_checks_unused_modules_and_pruned_direction_types() {
    assert!(!check("sheet component(a: i32, b: i32) { relationship { a := true; b := a; } } sheet root { source a: i32; cell b: i32; relationship component(a: a,b: b); }").is_empty());
    let path: ModulePath = "selected".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(path.clone(), "selected.adm2", "sheet good {}")
        .unwrap();
    modules
        .insert_source(
            "unused".parse().unwrap(),
            "unused.adm2",
            "sheet bad(a: Unknown) {}",
        )
        .unwrap();
    let errors = check_module(&path, &modules, &TypeRegistry::new(), &OpLookup::new());
    assert!(!errors.is_empty());
    assert_eq!(
        errors[0].labels[0]
            .location
            .as_ref()
            .unwrap()
            .source
            .file_name(),
        "unused.adm2"
    );
}

/// Retains the outer source-binding invocation in a transitive specialization diagnostic.
#[test]
fn module_analysis_labels_transitive_source_bindings() {
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            "leaf".parse().unwrap(),
            "leaf.adm2",
            "sheet copy(a: i32, b: i32) { relationship { a := b; b := a; } }",
        )
        .unwrap();
    modules
        .insert_source(
            "middle".parse().unwrap(),
            "middle.adm2",
            "use leaf::copy; sheet wrapper(a: i32, b: i32) { relationship copy(a: a,b: b); }",
        )
        .unwrap();
    let path: ModulePath = "outer".parse().unwrap();
    modules.insert_source(path.clone(), "outer.adm2", "use middle::wrapper; sheet root { source x: i32; source y: i32; relationship wrapper(a: x,b: y); }").unwrap();
    let errors = check_module(&path, &modules, &TypeRegistry::new(), &OpLookup::new());
    assert!(!errors.is_empty());
    let files: Vec<_> = errors
        .iter()
        .flat_map(|error| &error.labels)
        .filter_map(|label| {
            label
                .location
                .as_ref()
                .map(|location| location.source.file_name())
        })
        .collect();
    assert!(files.contains(&"outer.adm2"), "{errors:?}");
    assert!(files.contains(&"middle.adm2"), "{errors:?}");
    assert!(files.contains(&"leaf.adm2"), "{errors:?}");
}

/// Checks local contracts without requiring a component to admit an isolated solution.
#[test]
fn module_analysis_does_not_run_an_isolated_planner() {
    assert!(
        check(
            "sheet incomplete(a: i32,b: i32) { relationship { a := b; } relationship { a := b; } }"
        )
        .is_empty()
    );
}
