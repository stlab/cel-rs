# Collections

Arrays are non-empty, homogeneous, and recursively typed.

```rust
use cel_parser::{CELParser, OpLookup};
use cel_runtime::DynamicArray;

let mut segment = CELParser::new(OpLookup::new())
    .parse_str("[0i32, 1i32, 2i32]")
    .unwrap();

let array: DynamicArray = segment.call0().unwrap();
assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![0, 1, 2]);
```

The checked examples in
[cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs)
cover the basic array round-trip and the heterogeneous-array rejection case.

## What arrays accept

- Each element can itself be any expression, including a conditional or a
  nested array.
- Nested arrays are rank-one arrays whose element type is `DynamicArray`.
- The parser preserves element order.

```rust
use cel_parser::{CELParser, OpLookup};
use cel_runtime::DynamicArray;

let mut segment = CELParser::new(OpLookup::new())
    .parse_str("[[0i32], [1i32]]")
    .unwrap();

let array: DynamicArray = segment.call0().unwrap();
let rows = array.try_into_vec::<DynamicArray>().unwrap();
assert_eq!(rows[0].try_as_slice::<i32>().unwrap(), &[0]);
assert_eq!(rows[1].try_as_slice::<i32>().unwrap(), &[1]);
```

## What arrays do not accept

- Empty arrays are rejected, because this implementation does not infer an
  element type from `[]`.
- Trailing commas are rejected: `[0i32,]` is not a valid array literal.
- Mixed element types are rejected.
- Tuples are not valid array elements.

Those limitations are intentional and are covered by the parser tests.

## Tuples versus arrays

Parenthesized tuples and bracketed arrays are different runtime shapes:

- `(a, b)` is a tuple or grouping expression.
- `[a, b]` is a homogeneous array.

This distinction matters because tuple indexing and array element typing use
different runtime machinery.
