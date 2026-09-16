# Casts and closures

`as` casts are part of the parser's precedence chain and associate left to
right.

```rust
use cel_parser::{CELParser, OpLookup};

let mut segment = CELParser::new(OpLookup::new())
    .parse_str("1.5f64 as i32 as f64")
    .unwrap();

assert_eq!(segment.call0::<f64>().unwrap(), 1.0);
```

The checked parser tests cover cast execution and the fact that casts bind
tighter than unary negation.

## Closure syntax

Closures use pipe-delimited parameter lists:

```text
|| body
|x: i32| body
|a: i32, b: i32| body
|r: (i32, i32)| body
```

The parameter type after `:` can be a scalar name or a tuple type. The body is
an ordinary expression.

```rust
use cel_parser::{CELParser, OpLookup};
use cel_runtime::DynClosure;

let mut segment = CELParser::new(OpLookup::new())
    .parse_str("|x: i32| x + 1")
    .unwrap();

let closure: DynClosure = segment.call0().unwrap();
let x = 5i32;
assert_eq!(closure.call::<i32>(&[&x]).unwrap(), 6);
```

The checked examples and parser tests also cover:

- zero-parameter closures (`|| 42`);
- multiple parameters;
- tuple-typed parameters;
- closure bodies that return arrays or ranges;
- the fact that closures do not capture names from surrounding scopes.

## Practical limits

Closures are first-class runtime values, but they are still compiled from a
single expression body. They do not introduce block statements, mutable local
bindings, or captured environments in this implementation.

For the exact closure grammar and the isolation rules used during parsing, see
[cel-parser/src/lib.rs](https://github.com/stlab/cel-rs/blob/main/cel-parser/src/lib.rs)
and the checked parser tests.
