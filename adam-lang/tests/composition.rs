//! Exercises source-free composition through its public contracts.

use std::any::TypeId;
use std::cell::Cell;
use std::rc::Rc;

use adam_lang::type_registry::TypeShape;
use adam_lang::{CompositionBuilder, ParsedSheet, TypeRegistry};
use cel_runtime::{Array, ArrayOf, Cons, DynSegment, IntoCStackList, Leaf, Nil};

/// Compiles a collection and explicitly constructs its named root.
fn root(source: &str, name: &str) -> Result<ParsedSheet, adam_lang::ModuleDiagnostic> {
    let path = "app".parse().unwrap();
    let mut modules = adam_lang::ModuleSet::new();
    modules.insert_source(path, "app.adm2", source)?;
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .map_err(|mut errors| errors.remove(0))?;
    compiled.instantiate_root(&"app".parse().unwrap(), name)
}

/// Resolves exact CEL spans against their definition owner and only the failing use chain.
#[test]
fn composition_diagnostic_selects_definition_and_repeated_instance() {
    let definition = "sheet divide(a: i32, denominator: i32, b: i32) {
    relationship { a := b * denominator; b := 1 + a / denominator; }
}
sheet wrapper(a: i32, denominator: i32, b: i32) {
    relationship divide(a: a, denominator: denominator, b: b);
}";
    let invocation = "use library::wrapper;
sheet app {
    source a: i32 = 12; source good: i32 = 3; source bad: i32 = 0;
    cell first: i32; cell second: i32;
    relationship wrapper(a: a, denominator: good, b: first);
    relationship wrapper(a: a, denominator: bad, b: second);
}";
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source("library".parse().unwrap(), "library.adm2", definition)
        .unwrap();
    modules
        .insert_source("app".parse().unwrap(), "app.adm2", invocation)
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let definition_source = compiled
        .sources()
        .iter()
        .find(|s| s.file_name() == "library.adm2")
        .unwrap()
        .clone();
    let invocation_source = compiled
        .sources()
        .iter()
        .find(|s| s.file_name() == "app.adm2")
        .unwrap()
        .clone();
    let mut parsed = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap();
    drop(compiled);
    let error = parsed.propagate().unwrap_err();
    let labels = parsed.locate_error(&error);
    assert!(
        labels.iter().any(|label| label
            .location
            .as_ref()
            .is_some_and(|location| location.source == definition_source
                && location.span == cel_parser::SourceSpan::new(2, 50, 2, 65))),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label
            .location
            .as_ref()
            .is_some_and(
                |location| location.source == invocation_source && location.span.start.line == 6
            )),
        "{labels:?}"
    );
    assert!(
        !labels.iter().any(|label| label
            .location
            .as_ref()
            .is_some_and(
                |location| location.source == invocation_source && location.span.start.line == 5
            )),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label
            .location
            .as_ref()
            .is_some_and(
                |location| location.source == definition_source && location.span.start.line == 5
            )),
        "{labels:?}"
    );
    let adam_rs::Error::MethodFailed { error, .. } = error else {
        panic!("expected method failure")
    };
    assert!(error.downcast_ref::<cel_parser::SpanContext>().is_some());
}

/// Keeps source-free native diagnostics as labels without manufactured spans.
#[test]
fn composition_diagnostic_native_method_has_no_invented_span() {
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    builder.set_label(adam_lang::SourceLabel {
        location: None,
        label: "host native operation".into(),
    });
    let a = builder.parameter("a", int()).unwrap();
    let b = builder.parameter("b", int()).unwrap();
    let method = builder
        .method(std::slice::from_ref(&a), std::slice::from_ref(&b), || {
            let mut segment = DynSegment::new::<()>();
            segment.push_arg::<i32>(0);
            segment.op1r(|_: i32| -> anyhow::Result<i32> {
                Err(std::io::Error::other("native failure").into())
            })?;
            Ok(segment)
        })
        .unwrap();
    builder.relationship(vec![method]).unwrap();
    let template = builder.finish().unwrap();
    let mut parsed = ParsedSheet::new();
    let a = parsed.add_cell(1i32);
    let b = parsed.add_cell(0i32);
    parsed
        .register_cell("a", a, int(), &TypeRegistry::new())
        .unwrap();
    parsed
        .register_cell("b", b, int(), &TypeRegistry::new())
        .unwrap();
    template
        .instantiate(&mut parsed, &[("a", a), ("b", b)])
        .unwrap();
    let error = parsed.propagate().unwrap_err();
    let labels = parsed.locate_error(&error);
    assert!(!labels.is_empty());
    assert!(labels.iter().all(|label| label.location.is_none()));
    assert!(
        labels
            .iter()
            .any(|label| label.label.contains("host native operation")),
        "{labels:?}"
    );
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .is::<std::io::Error>()
    );
}

/// Selects only the failed imported native invocation without an inner CEL context.
#[test]
fn composition_diagnostic_imported_native_selects_actual_use() {
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let a = builder.parameter("a", int()).unwrap();
    let b = builder.parameter("b", int()).unwrap();
    let method = builder
        .method(&[a], &[b], || {
            let mut segment = DynSegment::new::<()>();
            segment.push_arg::<i32>(0);
            segment.op1r(|value: i32| -> anyhow::Result<i32> {
                if value == 0 {
                    Err(std::io::Error::other("native zero").into())
                } else {
                    Ok(value)
                }
            })?;
            Ok(segment)
        })
        .unwrap();
    builder.relationship(vec![method]).unwrap();
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_template("native".parse().unwrap(), "copy", builder.finish().unwrap())
        .unwrap();
    modules
        .insert_source(
            "app".parse().unwrap(),
            "caller.adm2",
            "use native::copy;
sheet app {
source good: i32 = 1; source bad: i32 = 0; cell first: i32; cell second: i32;
relationship copy(a: good, b: first);
relationship copy(a: bad, b: second);
}",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let source = compiled.sources()[0].clone();
    let mut parsed = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap();
    drop(compiled);
    let error = parsed.propagate().unwrap_err();
    let labels = parsed.locate_error(&error);
    assert!(labels[0].location.is_none(), "{labels:?}");
    let located: Vec<_> = labels
        .iter()
        .filter_map(|label| label.location.as_ref())
        .collect();
    assert_eq!(located.len(), 1, "{labels:?}");
    assert_eq!(located[0].source, source);
    assert_eq!(located[0].span.start.line, 5);
    let adam_rs::Error::MethodFailed { error, .. } = error else {
        panic!("expected native failure")
    };
    assert!(error.downcast_ref::<cel_parser::SpanContext>().is_none());
    assert!(error.downcast_ref::<std::io::Error>().is_some());
}

/// Retains the exact failing initializer definition and repeated construction path.
#[test]
fn composition_diagnostic_construction_selects_actual_use() {
    let mut lookup = cel_parser::OpLookup::new();
    let mut count = 0;
    thread_local! { static CALLS: Cell<usize> = const { Cell::new(0) }; }
    CALLS.with(|calls| calls.set(0));
    lookup.push_scope(|name, body, arity, _| {
        if name != "initial" || arity != 0 {
            return Ok(false);
        }
        body.op0r(|| -> anyhow::Result<i32> {
            CALLS.with(|calls| {
                let count = calls.get();
                calls.set(count + 1);
                if count == 1 {
                    Err(std::io::Error::other("second initializer").into())
                } else {
                    Ok(7)
                }
            })
        });
        Ok(true)
    });
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "definition.adm2",
            "sheet child() { cell value: i32 = initial; }",
        )
        .unwrap();
    modules
        .insert_source(
            "app".parse().unwrap(),
            "caller.adm2",
            "use library::child;
sheet app {
relationship child();
relationship child();
relationship child();
}",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), lookup)
        .compile(modules)
        .unwrap();
    let error = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap_err();
    drop(compiled);
    let uses: Vec<_> = error
        .labels
        .iter()
        .filter(|label| label.label == "component use")
        .filter_map(|label| label.location.as_ref())
        .collect();
    assert_eq!(uses.len(), 1, "{error:?}");
    assert_eq!(uses[0].span.start.line, 4);
    assert!(
        error.labels.iter().any(
            |label| label
                .location
                .as_ref()
                .is_some_and(|location| location.source.file_name() == "definition.adm2"
                    && location.span.start.column == 21)
        ),
        "{error:?}"
    );
    let mut cause = std::error::Error::source(&error);
    while let Some(error) = cause {
        if error.is::<std::io::Error>() {
            count += 1;
        }
        cause = error.source();
    }
    assert_eq!(count, 1);
}

/// Locates the actual failing source-mask preflight use before executing any factory.
#[test]
fn composition_diagnostic_preflight_selects_actual_use() {
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "definition.adm2",
            "sheet copy(a: i32, b: i32) { relationship { b := a; } }",
        )
        .unwrap();
    modules
        .insert_source(
            "app".parse().unwrap(),
            "caller.adm2",
            "use library::copy;
sheet app(a: i32, locked: i32, first: i32, third: i32) {
relationship copy(a: a, b: first);
relationship copy(a: a, b: locked);
relationship copy(a: a, b: third);
}",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let mut parsed = ParsedSheet::new();
    let a = parsed.add_source(1i32);
    let locked = parsed.add_source(0i32);
    let first = parsed.add_cell(0i32);
    let third = parsed.add_cell(0i32);
    let bindings = [
        ("a", a),
        ("locked", locked),
        ("first", first),
        ("third", third),
    ];
    for (name, id) in bindings {
        parsed
            .register_cell(name, id, int(), &TypeRegistry::new())
            .unwrap();
    }
    let error = compiled
        .template(&"app".parse().unwrap(), "app")
        .unwrap()
        .instantiate(&mut parsed, &bindings)
        .unwrap_err();
    let uses: Vec<_> = error
        .labels
        .iter()
        .filter(|label| label.label == "component use")
        .filter_map(|label| label.location.as_ref())
        .collect();
    assert_eq!(uses.len(), 1, "{error:?}");
    assert_eq!(uses[0].span.start.line, 4);
    assert!(
        error.labels.iter().any(
            |label| label
                .location
                .as_ref()
                .is_some_and(|location| location.source.file_name() == "definition.adm2"
                    && location.span.start.column == 29)
        ),
        "{error:?}"
    );
}

/// Resolves eager requirement evaluation spans against the actual policy definition.
#[test]
fn composition_diagnostic_requirement_construction_keeps_definition_span() {
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "policy.adm2",
            "sheet child() {
cell value: i32 = 1 require {
    value / 0 > 0;
};
}",
        )
        .unwrap();
    modules
        .insert_source(
            "app".parse().unwrap(),
            "app.adm2",
            "use library::child; sheet app { relationship child(); }",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let owner = compiled
        .sources()
        .iter()
        .find(|source| source.file_name() == "policy.adm2")
        .unwrap()
        .clone();
    let error = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap_err();
    drop(compiled);
    assert!(
        error.labels.iter().any(|label| label
            .location
            .as_ref()
            .is_some_and(|location| location.source == owner
                && location.span == cel_parser::SourceSpan::new(3, 4, 3, 13))),
        "{error:?}"
    );
}

/// Resolves private output-writer errors through their authored output expression.
#[test]
fn composition_diagnostic_output_writer_keeps_definition_span() {
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "output.adm2",
            "sheet child(a: i32) {
    out answer := 1 + 4 / a;
}",
        )
        .unwrap();
    modules
        .insert_source(
            "app".parse().unwrap(),
            "app.adm2",
            "use library::child; sheet app { source a: i32 = 0; relationship child(a: a); }",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let owner = compiled
        .sources()
        .iter()
        .find(|source| source.file_name() == "output.adm2")
        .unwrap()
        .clone();
    let mut parsed = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap();
    drop(compiled);
    let error = parsed.propagate().unwrap_err();
    let labels = parsed.locate_error(&error);
    assert!(
        labels.iter().any(|label| label
            .location
            .as_ref()
            .is_some_and(|location| location.source == owner
                && location.span == cel_parser::SourceSpan::new(2, 22, 2, 27))),
        "{labels:?}"
    );
    assert!(labels.iter().any(|label| label.label == "component use"));
}

/// Matches inline values, aspirations and diagnostics after identical public edits.
#[test]
fn composed_matches_manual_inline() {
    let declarations = "
        cell small: f64 = 2.0 filter 0.0..=100.0 require { @positive small > 0.0; };
        cell large: f64 = 4.0;
        source factor: f64 = 3.0;
        cell product: f64 = 0.0;
        cell other: f64 = 0.0;
        cell left: f64 = 5.0;
        cell right: f64 = 7.0;
        out answer := product + other;
    ";
    let components = "
        sheet ordered(lo: f64, hi: f64) {
            relationship { hi := lo + 2.0; lo := hi - 2.0; }
        }
        sheet multiply(a: f64, b: f64, c: f64) {
            relationship { c := a * b; a := c / b; b := c / a; }
        }
        sheet wrapper(a: f64, b: f64) {
            cell helper: f64 = 1.0;
            relationship { helper := a + 1.0; }
            relationship { b := helper * 2.0; }
        }
    ";
    let mut composed = root(
        &format!(
            "{components} sheet app {{ {declarations}
        relationship ordered(lo: small, hi: large);
        relationship multiply(a: small, b: factor, c: product);
        relationship multiply(a: left, b: right, c: other);
        relationship wrapper(a: large, b: left);
    }}"
        ),
        "app",
    )
    .unwrap();
    let mut manual = root(&format!("sheet app {{ {declarations}
        relationship {{ large := small + 2.0; small := large - 2.0; }}
        relationship {{ product := small * factor; small := product / factor; factor := product / small; }}
        relationship {{ other := left * right; left := other / right; right := other / left; }}
        cell helper: f64 = 1.0;
        relationship {{ helper := large + 1.0; }}
        relationship {{ left := helper * 2.0; }}
    }}"), "app").unwrap();
    assert!(!composed.cell_names.contains_key("helper"));
    // Construction must not propagate output writers or relationships.
    assert_eq!(
        *composed
            .read::<f64>(composed.cell_names["product"].0)
            .unwrap(),
        0.0
    );
    for edit in [
        None,
        Some(("small", 8.0)),
        Some(("right", 11.0)),
        Some(("small", -3.0)),
    ] {
        if let Some((name, value)) = edit {
            let composed_id = composed.cell_names[name].0;
            let manual_id = manual.cell_names[name].0;
            composed.write(composed_id, value).unwrap();
            manual.write(manual_id, value).unwrap();
        }
        assert_eq!(
            composed.propagate().map_err(|e| e.to_string()),
            manual.propagate().map_err(|e| e.to_string())
        );
        for name in [
            "small", "large", "factor", "product", "other", "left", "right", "answer",
        ] {
            let composed_value = composed.cell_names[name].0;
            let manual_value = manual.cell_names[name].0;
            assert_eq!(
                composed.read::<f64>(composed_value).unwrap(),
                manual.read::<f64>(manual_value).unwrap(),
                "{name}"
            );
            assert_eq!(
                composed.source::<f64>(composed_value).unwrap(),
                manual.source::<f64>(manual_value).unwrap(),
                "{name}"
            );
            assert_eq!(
                composed.is_source(composed_value),
                manual.is_source(manual_value),
                "{name}"
            );
            assert_eq!(
                composed.cell_requirements_valid(composed_value),
                manual.cell_requirements_valid(manual_value),
                "{name}"
            );
        }
    }
}

/// Allows transitive relationship-only uses and rejects private effects in inactive branches.
#[test]
fn conditional_uses_only_relationships() {
    let definitions = "
        sheet multiply(a: f64, b: f64, c: f64) { relationship { c := a * b; } }
        sheet wrapper(a: f64, b: f64, c: f64) { relationship multiply(a: a, b: b, c: c); }
    ";
    let body = "cell mode: bool = true; source a: f64 = 2.0; source b: f64 = 3.0; cell c: f64;
        conditional mode {
            true => { relationship wrapper(a: a, b: b, c: c); }
            _ => { relationship { c := 0.0; } }
        }";
    let mut parsed = root(&format!("{definitions} sheet app {{ {body} }}"), "app").unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(), 6.0);
    let mode = parsed.cell_names["mode"].0;
    parsed.write(mode, false).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(), 0.0);
    for effect in [
        "cell private: f64;",
        "source private: f64 = 1.0;",
        "out private := a;",
        "conditional true { true => { relationship { c := a; } } }",
    ] {
        let source = format!(
            "sheet child(a: f64, c: f64) {{ {effect} }}
            sheet wrapper(a: f64, c: f64) {{ relationship child(a: a, c: c); }}
            sheet app {{ source a: f64 = 2.0; cell c: f64;
                conditional true {{ true => {{ relationship {{ c := a; }} }}
                    _ => {{ relationship wrapper(a: a, c: c); }} }}
            }}"
        );
        let error = root(&source, "app").unwrap_err();
        assert!(
            error.labels.iter().any(|label| label.location.is_some()),
            "{error:?}"
        );
    }
}

/// Rejects invalid named actuals with owning locations before any initializer executes.
#[test]
fn root_binding_diagnostics_precede_live_construction() {
    for binding in [
        "missing(a: a)",
        "copy(a: unknown, b: b)",
        "copy(unknown: a, b: b)",
        "copy(a: a)",
        "copy(a: a, a: b)",
        "copy(a: a, b: a)",
        "copy(a: a, b: output)",
        "copy(a: a, b: b, extra: b)",
    ] {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let mut lookup = cel_parser::OpLookup::new();
        lookup.push_scope(move |name, segment, arity, _| {
            if name != "tick" || arity != 0 {
                return Ok(false);
            }
            let counter = counter.clone();
            segment.op0(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                1i32
            });
            Ok(true)
        });
        let path = "app".parse().unwrap();
        let mut modules = adam_lang::ModuleSet::new();
        modules
            .insert_source(
                path,
                "invalid.adm2",
                &format!(
                    "sheet copy(a: i32, b: i32) {{ relationship {{ b := a; }} }}
             sheet app {{ cell a: i32 = tick; cell b: i32; out output := a;
                relationship {binding}; }}"
                ),
            )
            .unwrap();
        let errors =
            match adam_lang::ModuleCompiler::new(TypeRegistry::new(), lookup).compile(modules) {
                Ok(_) => panic!("invalid binding accepted: {binding}"),
                Err(errors) => errors,
            };
        assert!(!errors.is_empty());
        assert!(
            errors
                .iter()
                .all(|error| error.labels.iter().any(|label| label.location.is_some()))
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}

/// Keeps explicit selection, output maps, root ordering and repeated parser ownership.
#[test]
fn root_selection_and_parser_share_common_lowering() {
    let source = "sheet copy(a: i32, b: i32) { relationship { b := a; } }
        sheet first { source input: i32 = 7; cell result: i32;
            relationship copy(a: input, b: result); out answer := result; }
        sheet second { cell own: i32 = 9; }";
    let mut parsed = root(source, "first").unwrap();
    assert_eq!(
        parsed
            .cell_names
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["input", "result", "answer"]
    );
    assert_eq!(parsed.output_names["answer"], parsed.cell_names["answer"].0);
    parsed.propagate().unwrap();
    assert_eq!(
        *parsed.read::<i32>(parsed.output_names["answer"]).unwrap(),
        7
    );
    assert!(root(source, "absent").is_err());
    let mut parser = adam_lang::AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new());
    let ambiguity = parser.parse_str(source).unwrap_err();
    let diagnostic = std::error::Error::source(&ambiguity)
        .unwrap()
        .downcast_ref::<adam_lang::ModuleDiagnostic>()
        .unwrap();
    assert_eq!(diagnostic.labels.len(), 2, "label every candidate root");
    assert!(diagnostic.labels.iter().all(|label| {
        label
            .location
            .as_ref()
            .is_some_and(|location| location.source.text() == source)
    }));
    let one = "sheet copy(a: i32, b: i32) { relationship { b := a; } }
        sheet app { source a: i32 = 4; cell b: i32; relationship copy(a: a, b: b); }";
    for _ in 0..2 {
        let mut parsed = parser.parse_str(one).unwrap();
        parsed.propagate().unwrap();
        assert_eq!(*parsed.read::<i32>(parsed.cell_names["b"].0).unwrap(), 4);
    }
    assert!(
        parser.parse_str("sheet helper() {}").is_err(),
        "no root must be rejected"
    );
}

/// Keeps the frontend's original owning construction diagnostic and native cause.
#[test]
fn parser_errors_retain_sources_and_native_causes() {
    use std::error::Error;
    let source = "sheet child() { cell bad: i32 = 1 / 0; }
        sheet app { relationship child(); }";
    let error = adam_lang::AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .parse_str(source)
        .unwrap_err();
    let copy = error.clone();
    drop(error);
    let diagnostic = copy
        .source()
        .unwrap()
        .downcast_ref::<adam_lang::ModuleDiagnostic>()
        .unwrap();
    assert!(diagnostic.labels.iter().any(|label| {
        label
            .location
            .as_ref()
            .is_some_and(|location| location.source.text() == source)
    }));
    assert!(diagnostic.source().unwrap().is::<adam_rs::TemplateError>());
}

/// Preserves lexical failure positions when no complete token stream exists.
#[test]
fn parser_syntax_errors_keep_lexical_locations() {
    use std::str::FromStr;
    for source in [
        "sheet app {\n cell x: i32;\n",
        "sheet app {\n cell x = \"unterminated;\n}",
    ] {
        let lexical = cel_parser::ParseError::from_lex_error(
            source,
            proc_macro2::TokenStream::from_str(source).unwrap_err(),
        );
        let error = adam_lang::AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
            .parse_str(source)
            .unwrap_err();
        assert_eq!(error.span().start(), lexical.span().start());
        assert_eq!(error.span().end(), lexical.span().end());
    }
}

/// Validates the final expanded graph without propagating or solving components alone.
#[test]
fn roots_validate_the_combined_guard_graph() {
    use std::error::Error;
    let error = root(
        "sheet loop(a: i32, b: i32) {
            conditional a { 0 => { relationship { a := b; b := a; } } }
        }
        sheet app { cell a: i32; cell b: i32; relationship loop(a: a, b: b); }",
        "app",
    )
    .unwrap_err();
    assert!(error.source().unwrap().is::<adam_rs::Error>());
    assert!(error.labels.iter().all(|label| label.location.is_some()));
}

/// Keeps imported relationship sites when combined root validation fails.
#[test]
fn root_validation_retains_imported_relationship_sources() {
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "library.adm2",
            "sheet copy(a: i32, b: i32) { relationship { b := a; } }",
        )
        .unwrap();
    let path = "app".parse().unwrap();
    modules
        .insert_source(
            path,
            "app.adm2",
            "use library::copy;
         sheet app { cell a: i32; cell b: i32; cell p: i32;
             relationship copy(a: a, b: p);
             conditional p { 0 => { relationship copy(a: b, b: a); } }
         }",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let error = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap_err();
    drop(compiled);
    for file in ["app.adm2", "library.adm2"] {
        assert!(
            error.labels.iter().any(|label| {
                label
                    .location
                    .as_ref()
                    .is_some_and(|location| location.source.file_name() == file)
            }),
            "{file}: {error:?}"
        );
    }
}

/// Preserves the actual imported conditional failure through public root construction.
#[test]
fn root_conditional_errors_keep_native_context_and_sources() {
    use std::error::Error;
    let mut lookup = cel_parser::OpLookup::new();
    lookup.push_scope(|name, segment, arity, _| {
        if name != "-" || arity != 1 {
            return Ok(false);
        }
        segment.op1r(|_: i32| -> anyhow::Result<i32> {
            Err(std::io::Error::other("deferred key failed").into())
        })?;
        Ok(true)
    });
    let mut modules = adam_lang::ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "library.adm2",
            "sheet child(a: i32, b: i32) {
            conditional a { -1 => { relationship { b := a; } } }
        }",
        )
        .unwrap();
    let path = "app".parse().unwrap();
    modules
        .insert_source(
            path,
            "app.adm2",
            "use library::child; sheet app {
            source a: i32 = 0; cell b: i32;
            relationship child(a: a, b: b);
        }",
        )
        .unwrap();
    let compiled = adam_lang::ModuleCompiler::new(TypeRegistry::new(), lookup)
        .compile(modules)
        .unwrap();
    let error = compiled
        .instantiate_root(&"app".parse().unwrap(), "app")
        .unwrap_err();
    drop(compiled);
    let native = error
        .source()
        .unwrap()
        .downcast_ref::<adam_rs::TemplateError>()
        .unwrap();
    assert!(
        matches!(native, adam_rs::TemplateError::ConditionalConstruction {
        conditional: 0, inclusions, ..
    } if inclusions.len() == 1)
    );
    for file in ["app.adm2", "library.adm2"] {
        assert!(error.labels.iter().any(|label| {
            label
                .location
                .as_ref()
                .is_some_and(|location| location.source.file_name() == file)
        }));
    }
    let mut cause = error.source();
    let mut found = false;
    while let Some(error) = cause {
        found |= error.is::<std::io::Error>();
        cause = error.source();
    }
    assert!(
        found,
        "the concrete deferred operation error remains downcastable"
    );
}

/// Returns the registered integer shape.
fn int() -> TypeShape {
    TypeShape::Named(TypeId::of::<i32>())
}

/// Emits a constant integer without evaluating it during definition checking.
fn zero() -> anyhow::Result<DynSegment> {
    let mut body = DynSegment::new::<()>();
    body.op0(|| 0i32);
    Ok(body)
}

#[test]
fn source_free_instances_keep_private_shapes_and_fresh_initializers() {
    let types = TypeRegistry::new();
    let calls = Rc::new(Cell::new(0usize));
    let mut builder = CompositionBuilder::new(types.clone());
    let count = calls.clone();
    let private = builder
        .cell("hidden", int(), move || {
            let count = count.clone();
            let mut body = DynSegment::new::<()>();
            body.op0(move || {
                count.set(count.get() + 1);
                i32::try_from(count.get()).unwrap()
            });
            Ok(body)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    assert_eq!(calls.get(), 0);
    let mut destination = ParsedSheet::new();
    let first = template.instantiate(&mut destination, &[]).unwrap();
    let second = template.instantiate(&mut destination, &[]).unwrap();
    let a = first
        .cell(first.occurrences()[0].id(), private.native())
        .unwrap();
    let b = second
        .cell(second.occurrences()[0].id(), private.native())
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(*destination.read::<i32>(a).unwrap(), 1);
    assert_eq!(*destination.read::<i32>(b).unwrap(), 2);
    assert_eq!(destination.cell_shape(a), Some(&int()));
    assert_eq!(
        destination.cell_origins[&a][0].occurrence,
        *first.occurrences()[0].id()
    );
    assert_eq!(&destination.cell_origins[&a][0].slot, private.native());
    assert!(destination.cell_names.is_empty());
    assert_eq!(destination.instance_origins.len(), 2);
    let mut consumer = CompositionBuilder::new(types);
    consumer.parameter("private_actual", int()).unwrap();
    consumer
        .finish()
        .unwrap()
        .instantiate(&mut destination, &[("private_actual", a)])
        .unwrap();
    assert_eq!(destination.cells().count(), 2);
    assert_eq!(destination.cell_origins[&a].len(), 2);
    drop(template);
    drop(first);
    drop(second);
    assert_eq!(destination.cell_shape(b), Some(&int()));
}

#[test]
fn binding_rejections_leave_destination_unchanged() {
    let types = TypeRegistry::new();
    let mut builder = CompositionBuilder::new(types.clone());
    builder.parameter("a", int()).unwrap();
    builder.parameter("b", int()).unwrap();
    builder.cell("never", int(), zero).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let x = destination.add_cell(3i32);
    let y = destination.add_cell(false);
    destination.register_cell("x", x, int(), &types).unwrap();
    destination
        .register_cell("y", y, TypeShape::Named(TypeId::of::<bool>()), &types)
        .unwrap();
    for bindings in [
        vec![("a", x)],
        vec![("a", x), ("b", x)],
        vec![("a", x), ("b", y)],
        vec![("a", x), ("a", x)],
        vec![("unknown", x), ("b", y)],
    ] {
        let error = template
            .instantiate(&mut destination, &bindings)
            .unwrap_err();
        assert!(error.labels.iter().all(|label| label.location.is_none()));
        assert_eq!(destination.cells().count(), 2);
        assert!(destination.instance_origins.is_empty());
    }
}

#[test]
fn full_shapes_reject_same_carrier_arrays_and_tuples() {
    let types = TypeRegistry::new();
    let mut destination = ParsedSheet::new();
    let array = destination
        .add_compound_cell::<ArrayOf<Leaf<bool>>>("array", Array::from_vec(vec![true]), &types)
        .unwrap();
    let tuple = destination
        .add_compound_cell::<Cons<Leaf<bool>, Nil>>("tuple", (true,).into_c_stack_list(), &types)
        .unwrap();
    for (shape, actual) in [
        (TypeShape::Array(Box::new(int())), array),
        (TypeShape::Tuple(vec![int()]), tuple),
    ] {
        let mut builder = CompositionBuilder::new(types.clone());
        builder.parameter("x", shape).unwrap();
        let template = builder.finish().unwrap();
        assert!(
            template
                .instantiate(&mut destination, &[("x", actual)])
                .is_err()
        );
        assert_eq!(destination.cells().count(), 2);
    }
}

/// Rejects incompatible emitted arrays without reserving declarations or adding graph items.
#[test]
fn emitted_array_results_require_complete_declared_shapes() {
    let emit_ints = || {
        let mut body = DynSegment::new::<()>();
        body.op0_repr::<ArrayOf<Leaf<i32>>, _>(|| Array::from_vec(vec![7i32]))?;
        Ok(body)
    };
    let emit_bools = || {
        let mut body = DynSegment::new::<()>();
        body.op0_repr::<ArrayOf<Leaf<bool>>, _>(|| Array::from_vec(vec![true]))?;
        Ok(body)
    };
    let shape = TypeShape::Array(Box::new(int()));
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let mut destination = ParsedSheet::new();
    let existing = destination.add_cell(9i32);

    let error = builder
        .cell("output", shape.clone(), emit_bools)
        .unwrap_err();
    assert!(!error.message.is_empty());
    assert_eq!(destination.cells().count(), 1);
    assert_eq!(destination.relationships().count(), 0);

    let output = builder.cell("output", shape, emit_ints).unwrap();
    let error = builder
        .method(&[], std::slice::from_ref(&output), emit_bools)
        .err()
        .expect("declared [i32] output must reject an emitted [bool]");
    assert!(!error.message.is_empty());
    assert_eq!(destination.cells().count(), 1);
    assert_eq!(destination.relationships().count(), 0);

    let valid = builder
        .method(&[], std::slice::from_ref(&output), emit_ints)
        .unwrap();
    builder.relationship(vec![valid]).unwrap();
    let instance = builder
        .finish()
        .unwrap()
        .instantiate(&mut destination, &[])
        .unwrap();
    assert_eq!(destination.cells().count(), 2);
    assert_eq!(destination.relationships().count(), 1);
    let output = instance
        .cell(instance.occurrences()[0].id(), output.native())
        .unwrap();
    destination.propagate().unwrap();
    assert_eq!(
        destination
            .read_compound::<ArrayOf<Leaf<i32>>>(output)
            .unwrap()
            .as_slice(),
        &[7]
    );
    assert_eq!(*destination.read::<i32>(existing).unwrap(), 9);
}

#[test]
fn nested_bindings_compare_callable_signatures_and_reject_foreign_slots() {
    let mut child = CompositionBuilder::new(TypeRegistry::new());
    child
        .parameter("f", TypeShape::Callable(vec![int()], Box::new(int())))
        .unwrap();
    let child = child.finish().unwrap();
    let mut parent = CompositionBuilder::new(TypeRegistry::new());
    let wrong = parent
        .parameter("f", TypeShape::Callable(vec![], Box::new(int())))
        .unwrap();
    assert!(parent.include(&child, &[("f", wrong)]).is_err());
    let mut other = CompositionBuilder::new(TypeRegistry::new());
    let foreign = other.parameter("foreign", int()).unwrap();
    assert!(parent.method(&[], &[foreign], zero).is_err());
}

#[test]
fn unregistered_shapes_are_errors_at_public_boundaries() {
    let unknown = TypeShape::Array(Box::new(TypeShape::Named(TypeId::of::<Cell<usize>>())));
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    assert!(builder.parameter("x", unknown.clone()).is_err());
    assert!(builder.cell("x", unknown.clone(), zero).is_err());
    let mut destination = ParsedSheet::new();
    let x = destination.add_cell(0i32);
    assert!(
        destination
            .register_cell("x", x, unknown, &TypeRegistry::new())
            .is_err()
    );
    assert!(destination.cell_names.is_empty());
}

/// Represents an arbitrary registered host leaf.
#[derive(Clone, Debug, Default, PartialEq)]
struct Host(i32);

#[test]
fn registered_host_leaf_methods_survive_template_owner_drop() {
    let mut types = TypeRegistry::new();
    types.register::<Host>("Host");
    let shape = TypeShape::Named(TypeId::of::<Host>());
    let mut builder = CompositionBuilder::new(types.clone());
    let input = builder.parameter("input", shape.clone()).unwrap();
    let output = builder
        .cell("private", shape, || {
            let mut body = DynSegment::new::<()>();
            body.op0(|| Host(0));
            Ok(body)
        })
        .unwrap();
    let method = builder
        .method(&[input], std::slice::from_ref(&output), || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<Host>(0);
            body.op1r(|x: Host| {
                x.0.checked_add(1)
                    .map(Host)
                    .ok_or_else(|| anyhow::anyhow!("overflow"))
            })?;
            Ok(body)
        })
        .unwrap();
    builder.relationship(vec![method]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let x = destination.add_source(Host(4));
    destination
        .register_cell("x", x, TypeShape::Named(TypeId::of::<Host>()), &types)
        .unwrap();
    let instance = template
        .instantiate(&mut destination, &[("input", x)])
        .unwrap();
    drop(template);
    drop(types);
    destination.propagate().unwrap();
    let output = instance
        .cell(instance.occurrences()[0].id(), output.native())
        .unwrap();
    assert_eq!(destination.read::<Host>(output).unwrap(), &Host(5));
}

#[test]
fn all_private_body_operations_delegate_and_retain_nested_occurrences() {
    let mut child = CompositionBuilder::new(TypeRegistry::new());
    let input = child.parameter("input", int()).unwrap();
    let output = child.parameter("output", int()).unwrap();
    let method = child
        .method(&[input], &[output], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            Ok(body)
        })
        .unwrap();
    child.relationship(vec![method]).unwrap();
    let child = child.finish().unwrap();

    let mut parent = CompositionBuilder::new(TypeRegistry::new());
    let source = parent
        .source("source", int(), || {
            let mut body = DynSegment::new::<()>();
            body.op0(|| 7i32);
            Ok(body)
        })
        .unwrap();
    let value = parent.cell("value", int(), zero).unwrap();
    parent
        .filter(&value, &[], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            body.op1(|x: i32| x.max(0))?;
            Ok(body)
        })
        .unwrap();
    parent
        .require(&value, std::slice::from_ref(&value), || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            body.op1(|x: i32| x >= 0)?;
            Ok(body)
        })
        .unwrap();
    let included = parent
        .include(
            &child,
            &[("input", source.clone()), ("output", value.clone())],
        )
        .unwrap();
    parent
        .conditional(
            &[],
            || {
                let mut body = DynSegment::new::<()>();
                body.op0(|| true);
                Ok(body)
            },
            vec![(vec![true], included.relationships().to_vec())],
            vec![],
        )
        .unwrap();
    let out = parent.cell("out", int(), zero).unwrap();
    let writer = parent
        .method(&[value], std::slice::from_ref(&out), || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            Ok(body)
        })
        .unwrap();
    parent.out(writer).unwrap();
    let template = parent.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = template.instantiate(&mut destination, &[]).unwrap();
    assert_eq!(instance.occurrences().len(), 2);
    let out_id = instance
        .cell(instance.occurrences()[0].id(), out.native())
        .unwrap();
    drop(child);
    drop(template);
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(out_id).unwrap(), 7);
    assert!(destination.cell_names.is_empty());
    assert!(
        destination
            .method_origins
            .values()
            .all(|origin| instance.occurrence(&origin.occurrence).is_some())
    );
}

#[test]
fn canonical_defaults_range_filters_and_compound_conditionals() {
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let value = builder.default_cell("default", int()).unwrap();
    builder
        .filter(&value, &[], || {
            let mut body = DynSegment::new::<()>();
            body.op0(|| 2i32..=4i32);
            Ok(body)
        })
        .unwrap();
    builder
        .conditional_compound::<ArrayOf<Leaf<bool>>, _>(
            &[],
            || {
                let mut body = DynSegment::new::<()>();
                body.op0_repr::<ArrayOf<Leaf<bool>>, _>(|| Array::from_vec(vec![true]))?;
                Ok(body)
            },
            vec![(vec![Array::from_vec(vec![true])], vec![])],
            vec![],
        )
        .unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = template.instantiate(&mut destination, &[]).unwrap();
    let value = instance
        .cell(instance.occurrences()[0].id(), value.native())
        .unwrap();
    destination.write(value, -1i32).unwrap();
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(value).unwrap(), 2);
}

#[test]
fn requirement_rejects_an_unregistered_bool_result_without_panicking() {
    let mut types = TypeRegistry::new();
    types.register::<Host>("bool");
    let mut builder = CompositionBuilder::new(types);
    let cell = builder.cell("x", int(), zero).unwrap();
    assert!(
        builder
            .require(&cell, &[], || {
                let mut body = DynSegment::new::<()>();
                body.op0(|| true);
                Ok(body)
            })
            .is_err()
    );
}

#[test]
fn compound_methods_preserve_full_values_across_independent_instances() {
    type Tuple = Cons<Leaf<i32>, Cons<Leaf<bool>, Nil>>;
    let types = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![int(), TypeShape::Named(TypeId::of::<bool>())]);
    let mut builder = CompositionBuilder::new(types.clone());
    let input = builder.parameter("input", shape.clone()).unwrap();
    let output = builder.default_cell("private", shape.clone()).unwrap();
    let owners = Rc::new(std::cell::RefCell::new(Vec::new()));
    let emitted = owners.clone();
    let descriptor = cel_runtime::ValueType::tuple(vec![
        cel_runtime::AssociatedType {
            offset: 0,
            value_type: (types.get("i32").unwrap().value_type_fn)(),
        },
        cel_runtime::AssociatedType {
            offset: 0,
            value_type: (types.get("bool").unwrap().value_type_fn)(),
        },
    ])
    .unwrap();
    let method = builder
        .method(&[input], std::slice::from_ref(&output), move || {
            let owner = Rc::new(());
            emitted.borrow_mut().push(Rc::downgrade(&owner));
            let mut body = DynSegment::new::<()>();
            body.push_argument(0, descriptor.clone())?;
            body.op1_repr::<Tuple, Tuple, _>(move |tuple| {
                let _retained = &owner;
                tuple
            })?;
            Ok(body)
        })
        .unwrap();
    builder.relationship(vec![method]).unwrap();
    assert!(
        owners
            .borrow()
            .iter()
            .all(|owner| owner.upgrade().is_none())
    );
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let a = destination
        .add_compound_cell::<Tuple>("a", (1, true).into_c_stack_list(), &types)
        .unwrap();
    let b = destination
        .add_compound_cell::<Tuple>("b", (2, false).into_c_stack_list(), &types)
        .unwrap();
    let first = template
        .instantiate(&mut destination, &[("input", a)])
        .unwrap();
    let second = template
        .instantiate(&mut destination, &[("input", b)])
        .unwrap();
    let a_out = first
        .cell(first.occurrences()[0].id(), output.native())
        .unwrap();
    let b_out = second
        .cell(second.occurrences()[0].id(), output.native())
        .unwrap();
    let live: Vec<_> = owners
        .borrow()
        .iter()
        .filter_map(std::rc::Weak::upgrade)
        .collect();
    assert_eq!(live.len(), 2);
    assert!(!Rc::ptr_eq(&live[0], &live[1]));
    drop(live);
    drop(template);
    destination.propagate().unwrap();
    assert_eq!(
        destination.read_compound::<Tuple>(a_out).unwrap(),
        (1, true).into_c_stack_list()
    );
    assert_eq!(
        destination.read_compound::<Tuple>(b_out).unwrap(),
        (2, false).into_c_stack_list()
    );
    destination
        .write_compound::<Tuple>(a, (9, true).into_c_stack_list())
        .unwrap();
    destination.propagate().unwrap();
    assert_eq!(
        destination.read_compound::<Tuple>(a_out).unwrap(),
        (9, true).into_c_stack_list()
    );
    assert_eq!(
        destination.read_compound::<Tuple>(b_out).unwrap(),
        (2, false).into_c_stack_list()
    );
    drop(destination);
    assert!(
        owners
            .borrow()
            .iter()
            .all(|owner| owner.upgrade().is_none())
    );
}

#[test]
fn source_owners_and_parent_links_survive_all_definition_owners() {
    let mut sources = adam_lang::SourceMap::new();
    let first = sources.insert("first.adm2", "first source");
    let mut other_sources = adam_lang::SourceMap::new();
    let second = other_sources.insert("second.adm2", "second source");
    let mut child = CompositionBuilder::new(TypeRegistry::new());
    child.set_label(adam_lang::SourceLabel {
        location: Some(adam_lang::SourceLocation {
            source: first.clone(),
            span: cel_parser::SourceSpan::from_proc_macro2(proc_macro2::Span::call_site()),
        }),
        label: "child".into(),
    });
    let hidden = child.default_cell("hidden", int()).unwrap();
    let child = child.finish().unwrap();
    let mut parent = CompositionBuilder::new(TypeRegistry::new());
    parent.set_label(adam_lang::SourceLabel {
        location: Some(adam_lang::SourceLocation {
            source: second.clone(),
            span: cel_parser::SourceSpan::from_proc_macro2(proc_macro2::Span::call_site()),
        }),
        label: "parent".into(),
    });
    let inclusion = parent.include(&child, &[]).unwrap();
    let parent = parent.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = parent.instantiate(&mut destination, &[]).unwrap();
    let child_occurrence = &instance.occurrences()[1];
    let private = instance
        .cell(child_occurrence.id(), hidden.native())
        .unwrap();
    let records = &destination.instance_origins[0].occurrences;
    assert_eq!(records[1].parent.as_ref(), Some(&records[0].id));
    assert_eq!(
        records[1].inclusion_site.as_ref(),
        Some(&inclusion.relationships()[0])
    );
    drop(parent);
    drop(child);
    drop(sources);
    drop(other_sources);
    drop(instance);
    assert_eq!(destination.cell_shape(private), Some(&int()));
    assert_eq!(destination.sources.len(), 2);
    assert!(destination.sources.contains(&first));
    assert!(destination.sources.contains(&second));
    assert_eq!(first.text(), "first source");
    assert!(destination.cell_names.is_empty());
}

#[test]
fn factory_failures_leave_destination_safely_discardable() {
    let calls = Rc::new(Cell::new(0usize));
    let attempts = calls.clone();
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let value = builder.default_cell("value", int()).unwrap();
    let writer = builder
        .method(&[], &[value], move || {
            attempts.set(attempts.get() + 1);
            if attempts.get() > 1 {
                anyhow::bail!("body factory failed");
            }
            zero()
        })
        .unwrap();
    builder.relationship(vec![writer]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let error = template.instantiate(&mut destination, &[]).unwrap_err();
    assert!(error.to_string().contains("body factory failed"));
    assert_eq!(destination.cells().count(), 1);
    assert!(destination.instance_origins.is_empty());
    drop(destination);

    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    builder
        .cell("fails", int(), || {
            let mut body = DynSegment::new::<()>();
            body.op0r(|| -> anyhow::Result<i32> { anyhow::bail!("initializer failed") });
            Ok(body)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    assert!(
        template
            .instantiate(&mut destination, &[])
            .unwrap_err()
            .to_string()
            .contains("initializer failed")
    );
    drop(destination);
}

#[test]
fn native_kind_and_source_specialization_rejection_precedes_initialization() {
    let types = TypeRegistry::new();
    let mut builder = CompositionBuilder::new(types.clone());
    let output = builder.parameter("output", int()).unwrap();
    let calls = Rc::new(Cell::new(0usize));
    let initializers = calls.clone();
    builder
        .cell("private", int(), move || {
            let initializers = initializers.clone();
            let mut body = DynSegment::new::<()>();
            body.op0(move || {
                initializers.set(initializers.get() + 1);
                0i32
            });
            Ok(body)
        })
        .unwrap();
    let writer = builder.method(&[], &[output], zero).unwrap();
    builder.relationship(vec![writer]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let source = destination.add_source(3i32);
    destination
        .register_cell("source", source, int(), &types)
        .unwrap();
    assert!(
        template
            .instantiate(&mut destination, &[("output", source)])
            .is_err()
    );
    assert_eq!(destination.cells().count(), 1);
    assert_eq!(calls.get(), 0);
    let out = destination.add_cell(0i32);
    destination
        .add_out(
            adam_rs::Method::from_fn_1_1(source, out, |x: &i32| Ok(*x)),
            vec![],
        )
        .unwrap();
    destination
        .register_cell("out", out, int(), &types)
        .unwrap();
    assert_eq!(destination.output_names["out"], out);
    assert!(
        template
            .instantiate(&mut destination, &[("output", out)])
            .is_err()
    );
    assert_eq!(destination.cells().count(), 2);
    assert_eq!(calls.get(), 0);
}

#[test]
fn deep_last_owner_metadata_teardown_is_iterative() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut template = CompositionBuilder::new(TypeRegistry::new())
                .finish()
                .unwrap();
            for _ in 0..4000 {
                let mut parent = CompositionBuilder::new(TypeRegistry::new());
                parent.include(&template, &[]).unwrap();
                template = parent.finish().unwrap();
            }
            drop(template);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn multi_output_source_specialization_retains_authored_method_indices() {
    let types = TypeRegistry::new();
    let mut builder = CompositionBuilder::new(types.clone());
    let a = builder.parameter("a", int()).unwrap();
    let b = builder.parameter("b", int()).unwrap();
    let c = builder.parameter("c", int()).unwrap();
    let pair = builder
        .method(std::slice::from_ref(&a), &[b.clone(), c.clone()], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            body.push_arg::<i32>(0);
            body.make_tuple(2, 0)?;
            Ok(body)
        })
        .unwrap();
    let inverse = builder
        .method(&[b, c], &[a], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            Ok(body)
        })
        .unwrap();
    let relationship = builder.relationship(vec![pair, inverse]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let a = destination.add_cell(0i32);
    let b = destination.add_source(3i32);
    let c = destination.add_cell(8i32);
    for (name, id) in [("a", a), ("b", b), ("c", c)] {
        destination.register_cell(name, id, int(), &types).unwrap();
    }
    let instance = template
        .instantiate(&mut destination, &[("a", a), ("b", b), ("c", c)])
        .unwrap();
    let group = instance
        .relationship_group(instance.occurrences()[0].id(), &relationship)
        .unwrap();
    assert_eq!(group.len(), 1);
    assert_eq!(destination.method_origins[&(group[0], 0)].authored_index, 1);
    assert!(instance.method_origin(group[0], 1).is_none());
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(a).unwrap(), 3);
    assert_eq!(*destination.read::<i32>(c).unwrap(), 8);
    let a2 = destination.add_source(5i32);
    let b2 = destination.add_cell(0i32);
    let c2 = destination.add_cell(0i32);
    for (name, id) in [("a2", a2), ("b2", b2), ("c2", c2)] {
        destination.register_cell(name, id, int(), &types).unwrap();
    }
    template
        .instantiate(&mut destination, &[("c", c2), ("a", a2), ("b", b2)])
        .unwrap();
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(b2).unwrap(), 5);
    assert_eq!(*destination.read::<i32>(c2).unwrap(), 5);
}

#[test]
fn registered_host_leaf_compounds_keep_canonical_capabilities() {
    let mut types = TypeRegistry::new();
    types.register::<Host>("Host");
    let mut builder = CompositionBuilder::new(types);
    let shape = TypeShape::Array(Box::new(TypeShape::Named(TypeId::of::<Host>())));
    let private = builder
        .source("hosts", shape.clone(), || {
            let mut body = DynSegment::new::<()>();
            body.op0_repr::<ArrayOf<Leaf<Host>>, _>(|| Array::from_vec(vec![Host(7)]))?;
            Ok(body)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = template.instantiate(&mut destination, &[]).unwrap();
    let cell = instance
        .cell(instance.occurrences()[0].id(), private.native())
        .unwrap();
    drop(template);
    assert_eq!(destination.cell_shape(cell), Some(&shape));
    assert_eq!(
        destination
            .read_compound::<ArrayOf<Leaf<Host>>>(cell)
            .unwrap()
            .as_slice(),
        &[Host(7)]
    );
}
