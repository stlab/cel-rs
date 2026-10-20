//! Contract tests for registry-backed expression checking in every sheet context.

use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
use cel_parser::{OpLookup, OperationSignature, RecordField, RecordSchema, ResolvedLeafType};
use cel_runtime::ValueType;

/// A host-defined record with no core-language special treatment.
#[derive(Clone, Debug, Default, PartialEq)]
struct TestPoint {
    x: f64,
}

/// A distinct host-defined leaf with an identical representation.
#[derive(Clone, Debug, Default, PartialEq)]
struct OtherPoint {
    x: f64,
}

/// Returns a concrete registered leaf.
fn leaf<T: 'static>(name: &'static str) -> ResolvedLeafType {
    ResolvedLeafType::new(name, ValueType::leaf::<T>())
}

/// Installs real constructors and accessors and their corresponding type annotations.
fn environment() -> (TypeRegistry, OpLookup) {
    let mut registry = TypeRegistry::new();
    registry.register::<TestPoint>("TestPoint");
    registry.register::<OtherPoint>("OtherPoint");
    let mut lookup = OpLookup::new();
    lookup
        .register_record(RecordSchema {
            ty: leaf::<TestPoint>("TestPoint"),
            fields: vec![RecordField {
                name: "x",
                ty: leaf::<f64>("f64").into(),
            }],
            emit: Box::new(|s, _, _, _| s.op1(|x: f64| TestPoint { x })),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "x",
            operands: vec![leaf::<TestPoint>("TestPoint").into()],
            result: leaf::<f64>("f64").into(),
            emit: Box::new(|s, _| s.op1(|p: TestPoint| p.x)),
        })
        .unwrap();
    (registry, lookup)
}

/// Checks a sheet against the same metadata a host uses to build it.
fn diagnostics(source: &str) -> Vec<cel_parser::ParseError> {
    let (registry, lookup) = environment();
    let sheet = AdamAstParser::new().parse_str(source).unwrap();
    assert!(sheet.errors.is_empty(), "{:?}", sheet.errors);
    check_sheet(&sheet, &registry, &lookup)
}

/// Rejects unrelated known types at annotated output, binding, and filter boundaries.
#[test]
fn registered_annotations_reject_unrelated_results() {
    for source in [
        "sheet s { cell p: TestPoint; out bad: OtherPoint := p; }",
        "sheet s { cell p: TestPoint; cell q: OtherPoint; relationship { q := p; } }",
        "sheet s { cell p: TestPoint filter 1.0; }",
        "sheet s { cell p: TestPoint; out bad: f64 := p; }",
        "sheet s { cell p: OtherPoint = TestPoint { x: 1.0 }; }",
        "sheet s { cell p: f64 = TestPoint { x: 1.0 }; }",
    ] {
        let errors = diagnostics(source);
        assert!(!errors.is_empty(), "{source}");
        assert!(
            errors.iter().any(|e| e.message().contains("TestPoint")),
            "{errors:?}"
        );
    }
}

/// Carries constructor and member metadata through nested sheet expressions.
#[test]
fn registered_results_are_checked_in_every_expression_context() {
    for source in [
        "sheet s { cell p: TestPoint = TestPoint { x: 1 }; }",
        "sheet s { cell p = TestPoint { x: 1 }; }",
        "sheet s { source p = Missing { x: true + 1 }; }",
        "sheet s { cell p: Missing = TestPoint { x: 1 }; }",
        "sheet s { cell p: TestPoint; out bad := p.x + true; }",
        "sheet s { out bad := |TestPoint: bool, x: bool| TestPoint { x: 1.0 }.x + true; }",
        "sheet s { cell p: TestPoint require { p.x; }; }",
        "sheet s { conditional (Missing {}).x { _ => {} } }",
    ] {
        assert!(!diagnostics(source).is_empty(), "{source}");
    }
}

/// Retains nested expression errors even when an enclosing output has the wrong shape.
#[test]
fn shape_errors_do_not_hide_child_diagnostics() {
    for source in [
        "sheet s { out result: (f64, f64) := (Missing {},); }",
        "sheet s { out result: (f64, f64) := Missing { x: 1 + true }; }",
    ] {
        let errors = diagnostics(source);
        assert!(
            errors.iter().any(|e| e.message().contains("Missing")),
            "{errors:?}"
        );
    }
}

/// Separates value names from type/constructor names and direct member dispatch.
#[test]
fn registered_member_resolution_ignores_same_named_values() {
    assert!(
        diagnostics(
            "sheet s { cell TestPoint: bool = true; cell x: bool = false; \
         cell p: TestPoint = TestPoint { x: 1.0 }; out value: f64 := p.x(); }"
        )
        .is_empty()
    );
}

/// Recursive cell annotations and dependencies retain exact leaf and tuple identities.
#[test]
fn compound_annotations_are_checked_in_every_sheet_context() {
    for source in [
        "sheet s { cell xs: [(TestPoint, [f64])] = [(TestPoint { x: 1.0 }, [2.0])]; out ys: [(TestPoint, [f64])] := xs; }",
        "sheet s { source xs: [(TestPoint, [f64])]; cell ys: [(TestPoint, [f64])]; relationship { ys := xs; } }",
        "sheet s { cell xs: [(TestPoint, [f64])] filter _; }",
        "sheet s { cell f: Fn((i32,), [TestPoint]) = |n: i32| [TestPoint { x: 1.0 }]; }",
    ] {
        assert!(
            diagnostics(source).is_empty(),
            "{source}: {:?}",
            diagnostics(source)
        );
    }
    for source in [
        "sheet s { cell xs: [(TestPoint, [f64])]; out ys: [(OtherPoint, [f64])] := xs; }",
        "sheet s { cell xs: [(TestPoint, [f64])]; cell ys: [(TestPoint, [i32])]; relationship { ys := xs; } }",
        "sheet s { cell xs: [(TestPoint, [f64])] filter [(OtherPoint { x: 1.0 }, [2.0])]; }",
        "sheet s { cell f: Fn((i32,), [OtherPoint]) = |n: i32| [TestPoint { x: 1.0 }]; }",
    ] {
        let errors = diagnostics(source);
        assert!(!errors.is_empty(), "{source}");
        assert!(
            errors.iter().all(|error| error.span().start().column > 0),
            "{errors:?}"
        );
    }
}
