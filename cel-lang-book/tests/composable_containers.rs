//! Executable contracts included by the collections and closures chapters.
// ANCHOR: tuple-arrays
/// Moves concrete tuple-list elements out of an interpreted array in source order.
#[test]
fn arrays_of_tuple_lists() {
    use cel_parser::{CELParser, OpLookup};
    use cel_runtime::{ArrayOf, Cons, IntoCStackList, IntoRustTuple, Leaf, Nil};

    type Row = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("[(7, [1.5, 2.5]), (8, []: [f64])]")
        .unwrap();
    let mut body = body.prepare::<Nil, ArrayOf<Row>>().unwrap();
    let rows = body.call(&().into_c_stack_list()).unwrap();
    let actual: Vec<_> = rows
        .into_vec()
        .into_iter()
        .map(|row| {
            let (id, values) = row.into_rust_tuple();
            (id, values.into_vec())
        })
        .collect();
    assert_eq!(actual, [(7, vec![1.5, 2.5]), (8, vec![])]);
}
// ANCHOR_END: tuple-arrays

// ANCHOR: nested-vector-view
/// Transfers the outer allocation to a vector of canonical inner arrays.
#[test]
fn nested_array_vector_view() {
    use cel_parser::{CELParser, OpLookup};
    use cel_runtime::{Array, ArrayOf, IntoCStackList, Leaf, Nil};

    let body = CELParser::new(OpLookup::new())
        .parse_str("[[1, 2], []: [i32], [3]]")
        .unwrap();
    let mut body = body.prepare::<Nil, ArrayOf<ArrayOf<Leaf<i32>>>>().unwrap();
    let rows: Array<Array<i32>> = body.call(&().into_c_stack_list()).unwrap();
    let pointer = rows.as_slice().as_ptr();
    let capacity = rows.capacity();
    let rows: Vec<Array<i32>> = rows.into_vec();
    assert_eq!(rows.as_ptr(), pointer);
    assert_eq!(rows.capacity(), capacity);
    assert_eq!(rows[0].as_slice(), [1, 2]);
    assert!(rows[1].is_empty());
    assert_eq!(rows[2].as_slice(), [3]);

    // Deep conversion moves each inner allocation, but builds a new outer vector.
    let native: Vec<Vec<i32>> = rows.into_iter().map(Array::into_vec).collect();
    assert_eq!(native, [vec![1, 2], vec![], vec![3]]);
}
// ANCHOR_END: nested-vector-view

// ANCHOR: callable-arrays
/// Preserves complete callable signatures even in empty arrays, without equality.
#[test]
fn typed_callable_arrays() {
    use cel_parser::{CELParser, OpLookup};
    use cel_runtime::{
        Array, ArrayOf, CNil, CStackList, Callable, Cons, FnOf, IntoCStackList, Leaf, Nil,
    };

    type Args = Cons<Leaf<i32>, Nil>;
    type Function = FnOf<Args, ArrayOf<Leaf<i32>>>;
    let mut parser = CELParser::new(OpLookup::new());
    let empty = parser.parse_str("[]: [Fn((i32,), [i32])]").unwrap();
    let mut empty = empty.prepare::<Nil, ArrayOf<Function>>().unwrap();
    assert!(empty.call(&().into_c_stack_list()).unwrap().is_empty());

    let body = parser.parse_str("[|x: i32| [x]]").unwrap();
    let mut body = body.prepare::<Nil, ArrayOf<Function>>().unwrap();
    let interpreted = body.call(&().into_c_stack_list()).unwrap();
    let native =
        Callable::from_fn(|args: &CStackList<i32, CNil<()>>| Ok(Array::from_vec(vec![args.1])));
    for function in [native, interpreted.into_vec().pop().unwrap()] {
        assert_eq!(
            function
                .call(&(42,).into_c_stack_list())
                .unwrap()
                .as_slice(),
            [42]
        );
    }
}
// ANCHOR_END: callable-arrays

// ANCHOR: shared-operation
use cel_runtime::{Array, CNil, CStackList};

/// Names the actual Rust value of a CEL `(i32, bool)`, not its witness.
type RowValue = CStackList<i32, CStackList<bool, CNil<()>>>;

/// Reverses rows without cloning their elements.
///
/// - Complexity: O(n) time for n rows; preserves the vector allocation.
fn reverse_rows(mut rows: Vec<RowValue>) -> Array<RowValue> {
    rows.reverse();
    Array::from_vec(rows)
}

/// Uses one concrete Rust operation from native construction and interpreted CEL.
#[test]
fn shared_native_and_interpreted_operation() {
    use cel_parser::{CELParser, OpLookup, OperationSignature, builtin_type_resolver};
    use cel_runtime::{ArrayOf, Cons, IntoCStackList, IntoRustTuple, Leaf, Nil};

    type Row = Cons<Leaf<i32>, Cons<Leaf<bool>, Nil>>;
    let native = reverse_rows(vec![
        (7, true).into_c_stack_list(),
        (8, false).into_c_stack_list(),
    ]);
    let shape = CELParser::new(OpLookup::new())
        .parse_type_expr_str("[(i32, bool)]")
        .unwrap()
        .resolve(builtin_type_resolver().as_ref())
        .unwrap();
    let mut lookup = OpLookup::new();
    /// Binds the operation's ordinary-call name separately from its signature.
    #[derive(Clone)]
    struct ReverseRows;
    lookup
        .register_function("reverse_rows", ReverseRows)
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "reverse_rows",
            operands: vec![shape.clone()],
            result: shape,
            emit: Box::new(|body, _| body.op1_vec::<Row, ArrayOf<Row>, _>(reverse_rows)),
        })
        .unwrap();
    let body = CELParser::new(lookup)
        .parse_str("reverse_rows([(7, true), (8, false)])")
        .unwrap();
    let mut body = body.prepare::<Nil, ArrayOf<Row>>().unwrap();
    for rows in [native, body.call(&().into_c_stack_list()).unwrap()] {
        let actual: Vec<_> = rows
            .into_vec()
            .into_iter()
            .map(IntoRustTuple::into_rust_tuple)
            .collect();
        assert_eq!(actual, [(8, false), (7, true)]);
    }
}
// ANCHOR_END: shared-operation

// ANCHOR: source-unit
/// Normalizes source unit to the zero-element list and bridges it to Rust unit.
#[test]
fn source_unit_is_an_empty_list() {
    use cel_parser::{CELParser, OpLookup};
    use cel_runtime::{CNil, IntoCStackList, IntoRustTuple, Nil};

    let body = CELParser::new(OpLookup::new()).parse_str("()").unwrap();
    let mut body = body.prepare::<Nil, Nil>().unwrap();
    let unit: CNil<()> = body.call(&().into_c_stack_list()).unwrap();
    let (): () = unit.into_rust_tuple();
}
// ANCHOR_END: source-unit
