//! Persistent compound cells preserve recursive ownership across propagation.
use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use cel_runtime::{Array, ArrayOf, Cons, IntoCStackList, Leaf, Nil};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A source-first integer paired with an owning array.
type Pair = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
/// An owning canonical array of recursively compound pairs.
type Pairs = ArrayOf<Pair>;

/// Resolves complete signatures through exactly the Adam registry used by execution.
fn shape(types: &TypeRegistry, source: &str) -> cel_parser::ResolvedType {
    cel_parser::CELParser::new(OpLookup::new())
        .parse_type_expr_str(source)
        .unwrap()
        .resolve(&types.cel_type_resolver())
        .unwrap()
}

/// Installs actual borrowed-slice requirements and a canonical owning-array filter.
fn operations(types: &TypeRegistry, calls: Arc<AtomicUsize>) -> OpLookup {
    let mut ops = OpLookup::new();
    ops.register_operation(cel_parser::OperationSignature {
        name: "nonempty",
        operands: vec![shape(types, "[(i32, [f64])]")],
        result: shape(types, "bool"),
        emit: Arc::new(move |body, _| {
            let calls = calls.clone();
            body.op1_slice::<Pair, Leaf<bool>, _>(move |values| {
                calls.fetch_add(1, Ordering::SeqCst);
                values.iter().all(|value| !value.0.1.is_empty())
            })
        }),
    })
    .unwrap();
    ops.register_operation(cel_parser::OperationSignature {
        name: "head",
        operands: vec![shape(types, "[(i32, [f64])]")],
        result: shape(types, "[(i32, [f64])]"),
        emit: Arc::new(|body, _| {
            body.op1_vec::<Pair, Pairs, _>(|mut values| {
                values.truncate(1);
                Array::from_vec(values)
            })
        }),
    })
    .unwrap();
    ops
}

/// Source writes retain independent aspirations and repeatedly update compound outputs.
#[test]
fn recursive_sources_update_twice() {
    let types = TypeRegistry::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut parser = AdamParser::new(types.clone(), operations(&types, calls.clone()));
    let mut sheet = parser
        .parse_str(
            "sheet s {
            source pairs: [(i32, [f64])] filter _.head;
            cell copy: [(i32, [f64])];
            relationship { copy := pairs; }
            out wrapped: ([(i32, [f64])],) := (copy,) require { @valid copy.nonempty; };
        }",
        )
        .unwrap();
    let pairs = sheet.cell_names["pairs"].0;
    let copy = sheet.cell_names["copy"].0;
    for n in [1i32, 2] {
        sheet
            .write_compound::<Pairs>(
                pairs,
                Array::from_vec(vec![
                    (n, Array::from_vec(vec![f64::from(n)])).into_c_stack_list(),
                    (99, Array::from_vec(vec![99.0])).into_c_stack_list(),
                ]),
            )
            .unwrap();
        sheet.propagate().unwrap();
        let original = sheet.read_compound::<Pairs>(pairs).unwrap();
        let result = sheet.read_compound::<Pairs>(copy).unwrap();
        assert_eq!(original.as_slice()[0].1, n);
        assert_eq!(result.as_slice()[0].1, n);
        assert_eq!(result.len(), 1);
        let aspiration = sheet.source_compound::<Pairs>(pairs).unwrap();
        assert_eq!(aspiration.len(), 2);
        assert_eq!(aspiration.as_slice()[1].1, 99);
        assert_ne!(
            aspiration.as_slice()[0].0.1.as_slice().as_ptr(),
            result.as_slice()[0].0.1.as_slice().as_ptr()
        );
        assert_ne!(original.as_slice().as_ptr(), result.as_slice().as_ptr());
        type Wrapped = Cons<Pairs, Nil>;
        let wrapped = sheet
            .read_compound::<Wrapped>(sheet.cell_names["wrapped"].0)
            .unwrap();
        assert_eq!(wrapped.1.as_slice()[0].1, n);
    }
    assert!(calls.load(Ordering::SeqCst) >= 2);
}

/// Empty arrays and recursive tuples default without needing element defaults.
#[test]
fn recursive_defaults_and_unit() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let sheet = parser.parse_str(
        "sheet s { cell xs: [(i32, [f64])]; cell t: (i32, [f64]); cell unit: (); cell one: ([i32],); }"
    ).unwrap();
    assert!(
        sheet
            .read_compound::<Pairs>(sheet.cell_names["xs"].0)
            .unwrap()
            .is_empty()
    );
    assert!(
        sheet
            .read_compound::<Pair>(sheet.cell_names["t"].0)
            .unwrap()
            .0
            .1
            .is_empty()
    );
    sheet
        .read_compound::<Nil>(sheet.cell_names["unit"].0)
        .unwrap();
}

thread_local! {
    /// Counts registered default effects independently for each test thread.
    static DEFAULT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A registered leaf whose default has an observable effect.
#[derive(Clone, Debug, PartialEq)]
struct Effect;

impl Default for Effect {
    /// Records one invocation of the registered default.
    fn default() -> Self {
        DEFAULT_CALLS.with(|calls| calls.set(calls.get().checked_add(1).unwrap()));
        Self
    }
}

/// Default construction validates every required capability before running any defaults.
#[test]
fn default_capabilities_reject_before_effects() {
    let mut types = TypeRegistry::new();
    types.register::<Effect>("Effect");
    for kind in ["cell", "source"] {
        for ty in [
            "([Fn((i32,),i32)],Effect)",
            "(Effect, ([Fn((i32,), i32)],))",
            "((Effect,), [Fn((i32,), i32)])",
        ] {
            DEFAULT_CALLS.with(|calls| calls.set(0));
            let source = format!("sheet s {{ {kind} x: {ty}; }}");
            let error = AdamParser::new(types.clone(), OpLookup::new())
                .parse_str(&source)
                .unwrap_err();
            assert!(error.message().contains("equality"), "{source}: {error}");
            assert!(error.span().start().column > 0);
            assert_eq!(DEFAULT_CALLS.with(|calls| calls.get()), 0, "{source}");
        }
    }
}

/// Missing defaults retain depth-first leaf/callable diagnostics without earlier effects.
#[test]
fn default_availability_rejects_before_effects() {
    let mut types = TypeRegistry::new();
    types.register::<Effect>("Effect");
    types.register_no_default::<std::ops::RangeInclusive<i32>>("Bounds(i32)");
    types.register_no_default::<std::ops::RangeInclusive<f64>>("Bounds(f64)");
    for kind in ["cell", "source"] {
        for (ty, diagnosis) in [
            (
                "(Effect, (i32, Bounds(i32)))",
                "type `Bounds(i32)` has no default",
            ),
            (
                "((Bounds(i32),), Effect)",
                "type `Bounds(i32)` has no default",
            ),
            ("(Effect, (Fn((i32,), i32),))", "callable has no default"),
            ("(Fn((i32,), i32), Effect)", "callable has no default"),
            (
                "([Fn((i32,), i32)], Effect, Bounds(i32))",
                "type `Bounds(i32)` has no default",
            ),
            (
                "(Effect, Bounds(i32), Fn((i32,), i32))",
                "type `Bounds(i32)` has no default",
            ),
            (
                "(Effect, Fn((i32,), i32), Bounds(i32))",
                "callable has no default",
            ),
            (
                "(Effect, (Bounds(i32), Bounds(f64)))",
                "type `Bounds(i32)` has no default",
            ),
            (
                "((Effect, Bounds(f64)), Bounds(i32))",
                "type `Bounds(f64)` has no default",
            ),
        ] {
            DEFAULT_CALLS.with(|calls| calls.set(0));
            let source = format!("sheet s {{ {kind} x: {ty}; }}");
            let error = AdamParser::new(types.clone(), OpLookup::new())
                .parse_str(&source)
                .unwrap_err();
            assert!(error.message().contains(diagnosis), "{source}: {error}");
            assert!(error.span().start().column > 0);
            assert_eq!(DEFAULT_CALLS.with(|calls| calls.get()), 0, "{source}");
        }
        DEFAULT_CALLS.with(|calls| calls.set(0));
        let source =
            format!("sheet s {{ {kind} x: (Effect, ([Bounds(i32)], Effect), [Effect]); }}");
        let sheet = AdamParser::new(types.clone(), OpLookup::new())
            .parse_str(&source)
            .unwrap();
        assert_eq!(DEFAULT_CALLS.with(|calls| calls.get()), 2, "{source}");
        type Defaults = Cons<
            Leaf<Effect>,
            Cons<
                Cons<ArrayOf<Leaf<std::ops::RangeInclusive<i32>>>, Cons<Leaf<Effect>, Nil>>,
                Cons<ArrayOf<Leaf<Effect>>, Nil>,
            >,
        >;
        let value = sheet
            .read_compound::<Defaults>(sheet.cell_names["x"].0)
            .unwrap();
        assert!(value.0.1.1.is_empty());
        assert!(value.0.0.1.is_empty());
    }
}

/// Callable cells reject absent equality rather than inventing function equality.
#[test]
fn callable_capabilities_are_explicit() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let missing = parser
        .parse_str("sheet s { cell f: Fn((i32,), i32); }")
        .unwrap_err();
    assert!(missing.message().contains("initializer") || missing.message().contains("provide"));
    let equality = parser
        .parse_str("sheet s { cell f: Fn((i32,), i32) = |x: i32| x; }")
        .unwrap_err();
    assert!(equality.message().contains("equality"), "{equality}");
    assert!(equality.span().start().column > 0);
    let array = parser
        .parse_str("sheet s { cell fs = [|x: i32| x]: [Fn((i32,), i32)]; }")
        .unwrap_err();
    assert!(array.message().contains("equality"), "{array}");
    let default_array = parser
        .parse_str("sheet s { cell fs: [Fn((i32,), i32)]; }")
        .unwrap_err();
    assert!(
        default_array.message().contains("equality"),
        "{default_array}"
    );
    assert!(default_array.span().start().column > 0);
    let mut cel = cel_parser::CELParser::new(OpLookup::new());
    let body = cel.parse_str("[|x: i32| x]: [Fn((i32,), i32)]").unwrap();
    type Functions = ArrayOf<cel_runtime::FnOf<Cons<Leaf<i32>, Nil>, Leaf<i32>>>;
    let mut prepared = body.prepare::<Nil, Functions>().unwrap();
    assert_eq!(prepared.call(&().into_c_stack_list()).unwrap().len(), 1);
}

/// Multi-output moves include tuple arrays, singleton tuples, and source unit.
#[test]
fn destructuring_moves_recursive_outputs() {
    let mut sheet = AdamParser::new(TypeRegistry::new(), OpLookup::new())
        .parse_str(
            "sheet s {
            source pairs: [(i32, [f64])] = [(1, [2.0])];
            cell left: [(i32, [f64])]; cell count: i32; cell unit: (); cell one: ([i32],);
            relationship { (left, count, unit, one) := (pairs, 7, (), ([9],)); }
        }",
        )
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(sheet.cell_names["count"].0).unwrap(), 7);
    assert_eq!(
        sheet
            .read_compound::<Pairs>(sheet.cell_names["left"].0)
            .unwrap()
            .as_slice()[0]
            .1,
        1
    );
    sheet
        .read_compound::<Nil>(sheet.cell_names["unit"].0)
        .unwrap();
    type One = Cons<ArrayOf<Leaf<i32>>, Nil>;
    assert_eq!(
        sheet
            .read_compound::<One>(sheet.cell_names["one"].0)
            .unwrap()
            .1
            .as_slice(),
        &[9]
    );
}

/// Records a unique identity for every constructed or cloned runtime owner.
#[derive(Debug, Default)]
struct OwnerLog {
    created: AtomicUsize,
    dropped: Mutex<Vec<usize>>,
}

impl OwnerLog {
    /// Returns the number of owners not yet destroyed, rejecting duplicate destruction.
    /// - Complexity: O(n log n) in destroyed owners.
    fn live(&self) -> usize {
        let mut dropped = self.dropped.lock().unwrap().clone();
        dropped.sort_unstable();
        assert!(dropped.windows(2).all(|ids| ids[0] != ids[1]));
        self.created
            .load(Ordering::SeqCst)
            .checked_sub(dropped.len())
            .unwrap()
    }

    /// Verifies that each constructed owner was destroyed exactly once.
    /// - Complexity: O(n log n) in constructed owners.
    fn assert_clean(&self) {
        let mut dropped = self.dropped.lock().unwrap().clone();
        dropped.sort_unstable();
        assert_eq!(
            dropped,
            (0..self.created.load(Ordering::SeqCst)).collect::<Vec<_>>()
        );
    }
}

/// A comparable leaf whose independent clones each require exactly one destruction.
#[derive(Debug)]
struct Tracked {
    value: i32,
    owner: usize,
    log: Arc<OwnerLog>,
}

impl Tracked {
    /// Constructs and records one distinct owner.
    fn new(value: i32, log: &Arc<OwnerLog>) -> Self {
        Self {
            value,
            owner: log.created.fetch_add(1, Ordering::SeqCst),
            log: log.clone(),
        }
    }
}

impl Clone for Tracked {
    /// Records an independently owned clone with the same logical value.
    fn clone(&self) -> Self {
        Self::new(self.value, &self.log)
    }
}

impl PartialEq for Tracked {
    /// Compares logical values rather than ownership identities.
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Drop for Tracked {
    /// Records exactly one destruction for this owner's unique identity.
    fn drop(&mut self) {
        self.log.dropped.lock().unwrap().push(self.owner);
    }
}

/// A tracked row owns a leaf and a nested owning array.
type TrackedRow = Cons<Leaf<Tracked>, Cons<ArrayOf<Leaf<Tracked>>, Nil>>;
/// The recursively owned input/output shape exercised by failing frames.
type TrackedRows = ArrayOf<TrackedRow>;

/// Builds two independently owned rows for a source aspiration.
fn tracked_rows(
    value: i32,
    log: &Arc<OwnerLog>,
) -> Array<<TrackedRow as cel_runtime::ValueRepr>::Value> {
    Array::from_vec(
        [value, 99]
            .into_iter()
            .map(|n| {
                (
                    Tracked::new(n, log),
                    Array::from_vec(vec![Tracked::new(n, log)]),
                )
                    .into_c_stack_list()
            })
            .collect(),
    )
}

/// Failed later tuple fields clean completed outputs and keep the sheet reusable.
#[test]
fn failing_multioutput_cleans_completed_compounds() {
    let log = Arc::new(OwnerLog::default());
    let mut types = TypeRegistry::new();
    types.register_no_default::<Tracked>("Tracked");
    let mut ops = OpLookup::new();
    ops.push_scope(|name, body, arity, _| {
        if name != "checked" || arity != 1 {
            return Ok(false);
        }
        body.op1r(|n: i32| {
            anyhow::ensure!(n >= 0, "later output rejected");
            Ok(n)
        })?;
        Ok(true)
    });
    let mut sheet = AdamParser::new(types, ops)
        .parse_str(
            "sheet s {
                source xs: [(Tracked, [Tracked])]; source gate: i32;
                cell left: [(Tracked, [Tracked])];
                cell right: ([(Tracked, [Tracked])],); cell number: i32;
                relationship { (left, right, number) := (xs, (xs,), gate.checked); }
            }",
        )
        .unwrap();
    let xs = sheet.cell_names["xs"].0;
    let left = sheet.cell_names["left"].0;
    let right = sheet.cell_names["right"].0;
    let gate = sheet.cell_names["gate"].0;
    let number = sheet.cell_names["number"].0;
    sheet
        .write_compound::<TrackedRows>(xs, tracked_rows(7, &log))
        .unwrap();
    sheet.propagate().unwrap();
    for _ in 0..2 {
        sheet.write(gate, -1i32).unwrap();
        let live = log.live();
        let created = log.created.load(Ordering::SeqCst);
        let error = sheet.propagate().unwrap_err();
        assert!(matches!(&error, adam_rs::Error::MethodFailed { .. }));
        assert!(
            error.to_string().contains("later output rejected"),
            "{error}"
        );
        assert!(log.created.load(Ordering::SeqCst) > created);
        assert_eq!(log.live(), live);
        let aspiration = sheet.source_compound::<TrackedRows>(xs).unwrap();
        assert_eq!(aspiration.len(), 2);
        assert_eq!(aspiration.as_slice()[0].1.value, 7);
        assert_eq!(aspiration.as_slice()[0].0.1.as_slice()[0].value, 7);
        assert_eq!(
            sheet.read_compound::<TrackedRows>(left).unwrap(),
            aspiration
        );
        type WrappedRows = Cons<TrackedRows, Nil>;
        assert_eq!(
            sheet.read_compound::<WrappedRows>(right).unwrap().1,
            aspiration
        );
        assert_eq!(*sheet.read::<i32>(number).unwrap(), 0);
        assert_eq!(sheet.changed().count(), 0);
    }
    sheet.write(gate, 3i32).unwrap();
    sheet
        .write_compound::<TrackedRows>(xs, tracked_rows(8, &log))
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(number).unwrap(), 3);
    assert_eq!(
        sheet.read_compound::<TrackedRows>(left).unwrap().as_slice()[0]
            .1
            .value,
        8
    );
    drop(sheet);
    log.assert_clean();
}

/// Failed owning filters clean candidates without consuming protected source aspirations.
#[test]
fn failing_filter_cleans_candidates_and_preserves_aspiration() {
    let log = Arc::new(OwnerLog::default());
    let mut types = TypeRegistry::new();
    types.register_no_default::<Tracked>("Tracked");
    let mut ops = OpLookup::new();
    ops.register_operation(cel_parser::OperationSignature {
        name: "conform",
        operands: vec![shape(&types, "[(Tracked, [Tracked])]")],
        result: shape(&types, "[(Tracked, [Tracked])]"),
        emit: Arc::new(|body, _| {
            body.op1r_vec::<TrackedRow, TrackedRows, _>(|mut rows| {
                rows.truncate(1);
                anyhow::ensure!(
                    rows.first().is_none_or(|row| row.1.value >= 0),
                    "candidate rejected"
                );
                Ok(Array::from_vec(rows))
            })
        }),
    })
    .unwrap();
    let mut sheet = AdamParser::new(types, ops)
        .parse_str(
            "sheet s {
                source xs: [(Tracked, [Tracked])] filter _.conform;
                out copy: [(Tracked, [Tracked])] := xs;
            }",
        )
        .unwrap();
    let xs = sheet.cell_names["xs"].0;
    let copy = sheet.cell_names["copy"].0;
    for value in [7, -1, -2, 8] {
        sheet
            .write_compound::<TrackedRows>(xs, tracked_rows(value, &log))
            .unwrap();
        sheet.propagate().unwrap();
        let aspiration = sheet.source_compound::<TrackedRows>(xs).unwrap();
        assert_eq!(aspiration.len(), 2);
        assert_eq!(aspiration.as_slice()[0].1.value, value);
        assert_eq!(aspiration.as_slice()[0].0.1.as_slice()[0].value, value);
        assert_eq!(aspiration.as_slice()[1].1.value, 99);
        let effective = sheet.read_compound::<TrackedRows>(xs).unwrap();
        assert_eq!(effective.len(), if value < 0 { 2 } else { 1 });
        assert_eq!(effective.as_slice()[0].1.value, value);
        assert_eq!(sheet.read_compound::<TrackedRows>(copy).unwrap(), effective);
        if value < 0 {
            let violation = sheet.filter_violation(xs).unwrap();
            assert!(matches!(violation, adam_rs::FilterViolation::Failed(_)));
            assert!(format!("{violation:?}").contains("candidate rejected"));
            let live = log.live();
            sheet.propagate().unwrap();
            assert_eq!(log.live(), live);
            assert_eq!(
                sheet.source_compound::<TrackedRows>(xs).unwrap(),
                aspiration
            );
            assert_eq!(sheet.read_compound::<TrackedRows>(xs).unwrap(), effective);
        } else {
            assert!(sheet.filter_violation(xs).is_none());
        }
        assert_ne!(
            aspiration.as_slice().as_ptr(),
            effective.as_slice().as_ptr()
        );
    }
    drop(sheet);
    log.assert_clean();
}

/// Generic foreign leaves need only one registration, not each containing shape.
#[test]
fn registered_generic_leaves_and_missing_defaults() {
    let mut types = TypeRegistry::new();
    types.register_no_default::<std::ops::RangeInclusive<i32>>("Bounds(i32)");
    let mut parser = AdamParser::new(types, OpLookup::new());
    let mut sheet = parser
        .parse_str(
            "sheet s {
            source xs: [(Bounds(i32), [i32])];
            out ys: [(Bounds(i32), [i32])] := xs;
        }",
        )
        .unwrap();
    type Item = Cons<Leaf<std::ops::RangeInclusive<i32>>, Cons<ArrayOf<Leaf<i32>>, Nil>>;
    type Items = ArrayOf<Item>;
    let xs = sheet.cell_names["xs"].0;
    sheet
        .write_compound::<Items>(
            xs,
            Array::from_vec(vec![(1..=3, Array::from_vec(vec![4])).into_c_stack_list()]),
        )
        .unwrap();
    sheet.propagate().unwrap();
    let ys = sheet
        .read_compound::<Items>(sheet.cell_names["ys"].0)
        .unwrap();
    assert_eq!(ys.as_slice()[0].1, 1..=3);
    let missing = parser
        .parse_str("sheet s { cell x: ([i32], Bounds(i32)); }")
        .unwrap_err();
    assert!(missing.message().contains("Bounds(i32)"), "{missing}");
    assert!(missing.message().contains("no default"), "{missing}");
    assert!(missing.span().start().column > 0);
}

/// Adding an argument alias preserves existing generic cells and compound propagation.
#[test]
fn registered_aliases_preserve_generic_sheet_construction() {
    use std::ops::RangeInclusive;

    let mut types = TypeRegistry::new();
    types.register_no_default::<RangeInclusive<i32>>("Bounds(i32)");
    for add_alias in [false, true] {
        if add_alias {
            types.register::<i32>("Integer");
        }
        let mut parser = AdamParser::new(types.clone(), OpLookup::new());
        let mut sheet = parser
            .parse_str(
                "sheet s {
                    cell bounds: Bounds(i32) = 0..=10;
                    source xs: [(Bounds(i32), [i32])];
                    out ys: [(Bounds(i32), [i32])] := xs;
                }",
            )
            .unwrap();
        assert_eq!(
            *sheet
                .read::<RangeInclusive<i32>>(sheet.cell_names["bounds"].0)
                .unwrap(),
            0..=10
        );
        /// A generic leaf beside a structural array.
        type Item = Cons<Leaf<RangeInclusive<i32>>, Cons<ArrayOf<Leaf<i32>>, Nil>>;
        sheet
            .write_compound::<ArrayOf<Item>>(
                sheet.cell_names["xs"].0,
                Array::from_vec(vec![(1..=3, Array::from_vec(vec![4])).into_c_stack_list()]),
            )
            .unwrap();
        sheet.propagate().unwrap();
        let ys = sheet
            .read_compound::<ArrayOf<Item>>(sheet.cell_names["ys"].0)
            .unwrap();
        assert_eq!(ys.as_slice()[0].1, 1..=3);
        assert_eq!(ys.as_slice()[0].0.1.as_slice(), &[4]);
    }
}

/// Complete mismatches fail before operand effects and before host writes.
#[test]
fn mismatches_reject_before_effects_or_writes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let effects = calls.clone();
    let mut ops = OpLookup::new();
    ops.push_scope(move |name, body, arity, _| {
        if name != "effect" || arity != 0 {
            return Ok(false);
        }
        let effects = effects.clone();
        body.op0(move || {
            effects.fetch_add(1, Ordering::SeqCst);
            1i32
        });
        Ok(true)
    });
    let mut parser = AdamParser::new(TypeRegistry::new(), ops);
    let error = parser
        .parse_str("sheet s { cell xs: [(i32, [f64])] = [(effect, [1])]; }")
        .unwrap_err();
    assert!(error.message().contains("type mismatch"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let error = parser
        .parse_str("sheet s { cell xs = [(effect, |x: i32| x)]; }")
        .unwrap_err();
    assert!(error.message().contains("equality"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut sheet = parser
        .parse_str("sheet s { source xs: [i32] = [4]; }")
        .unwrap();
    let xs = sheet.cell_names["xs"].0;
    assert!(
        sheet
            .write_compound::<ArrayOf<Leaf<u32>>>(xs, Array::from_vec(vec![5]))
            .is_err()
    );
    assert!(sheet.read_compound::<ArrayOf<Leaf<u32>>>(xs).is_err());
    assert_eq!(
        sheet
            .read_compound::<ArrayOf<Leaf<i32>>>(xs)
            .unwrap()
            .as_slice(),
        &[4]
    );
}

/// Zero-sized source unit remains distinct from a host unit leaf inside arrays and tuples.
#[test]
fn source_unit_composes_with_empty_and_singleton_shapes() {
    let mut sheet = AdamParser::new(TypeRegistry::new(), OpLookup::new()).parse_str(
        "sheet s { source units: [()] = [(), ()]; out nested: ([()], (), ((),)) := (units, (), ((),)); }"
    ).unwrap();
    sheet.propagate().unwrap();
    type Nested = Cons<ArrayOf<Nil>, Cons<Nil, Cons<Cons<Nil, Nil>, Nil>>>;
    let value = sheet
        .read_compound::<Nested>(sheet.cell_names["nested"].0)
        .unwrap();
    assert_eq!(value.1.len(), 2);
    assert!(
        sheet
            .read_compound::<ArrayOf<Leaf<()>>>(sheet.cell_names["units"].0)
            .is_err()
    );
}

/// Registered function markers preserve their registered equality and concrete access.
#[test]
fn registered_callable_markers_keep_concrete_capabilities() {
    /// A comparable host function marker, not invented closure identity.
    #[derive(Clone, Debug, PartialEq)]
    struct Marker;
    let mut types = TypeRegistry::new();
    types.register_no_default::<Marker>("Marker");
    let mut ops = OpLookup::new();
    ops.register_function("successor", Marker).unwrap();
    ops.register_operation(cel_parser::OperationSignature {
        name: "successor",
        operands: vec![shape(&types, "i32")],
        result: shape(&types, "i32"),
        emit: Arc::new(|body, _| {
            body.op1r(|n: i32| n.checked_add(1).ok_or_else(|| anyhow::anyhow!("overflow")))
        }),
    })
    .unwrap();
    let mut sheet = AdamParser::new(types, ops)
        .parse_str(
            "sheet s {
                source n: i32 = 1; cell f: Marker = successor; cell fs: [Marker] = [successor];
                out value: i32 := f(n); out from_array: [Marker] := fs;
            }",
        )
        .unwrap();
    let n = sheet.cell_names["n"].0;
    for value in [2i32, 3] {
        sheet.write(n, value).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(
            *sheet.read::<i32>(sheet.cell_names["value"].0).unwrap(),
            value.checked_add(1).unwrap()
        );
        assert_eq!(
            sheet
                .read_compound::<ArrayOf<Leaf<Marker>>>(sheet.cell_names["from_array"].0)
                .unwrap()
                .as_slice(),
            &[Marker]
        );
    }
    assert_eq!(
        *sheet.read::<Marker>(sheet.cell_names["f"].0).unwrap(),
        Marker
    );
}

/// Conditional subjects and requirement callbacks borrow the same recursive cell frames.
#[test]
fn compound_dependencies_drive_conditionals_and_requirements() {
    let types = TypeRegistry::new();
    let ops = operations(&types, Arc::new(AtomicUsize::new(0)));
    let mut sheet = AdamParser::new(types, ops)
        .parse_str(
            "sheet s {
            source pairs: [(i32, [f64])] require { @valid pairs.nonempty; };
            source a: i32 = 11; source b: i32 = 22; cell result: i32;
            conditional pairs.nonempty {
                true => { relationship { result := a; } }
                _ => { relationship { result := b; } }
            }
        }",
        )
        .unwrap();
    let pairs = sheet.cell_names["pairs"].0;
    let result = sheet.cell_names["result"].0;
    for (values, expected, valid) in [(vec![2.0], 11, true), (vec![], 22, false)] {
        sheet
            .write_compound::<Pairs>(
                pairs,
                Array::from_vec(vec![(1, Array::from_vec(values)).into_c_stack_list()]),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(result).unwrap(), expected);
        assert_eq!(sheet.cell_requirements_valid(pairs), valid);
    }
}
