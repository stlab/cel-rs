# Literals and types

CEL literals are ordinary Rust token literals fed through `proc_macro2` and
the `cel-parser` lexer. The parser accepts integer, float, boolean, string,
and character literals, then turns them into runtime values before execution.
That means the book can talk about a parsed expression as a `DynSegment`, and
about a runtime result as the value produced by `call0` or another call method.

```rust
use cel_parser::{CELParser, OpLookup};

let mut segment = CELParser::new(OpLookup::new()).parse_str("10u32").unwrap();
assert_eq!(segment.call0::<u32>().unwrap(), 10);
```

The checked examples in
[cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs)
exercise the same path for arithmetic, booleans, strings, and custom name
resolution.

## Literal forms

- Integer literals use Rust suffixes such as `i32`, `u32`, `f32`, and `f64`.
  Unsuffixed integers default to `i32`.
- Float literals accept the default `f64` form and explicit `f32`/`f64`
  suffixes.
- Boolean literals are `true` and `false`.
- String and character literals are passed through as runtime `String` and
  `char` values.

These forms are the ones demonstrated by the parser tests and by the
checked examples in the book.

## Identifiers and runtime values

An identifier is not automatically a variable. It becomes a runtime value only
if the active `OpLookup` resolves it, either through a built-in operation or a
custom scope.

```rust
use cel_parser::{CELParser, OpLookup};

let mut lookup = OpLookup::new();
lookup.push_scope(|name, segment, num_operands, _span| match (name, num_operands) {
    ("x", 0) => {
        segment.op0(|| 10i32);
        Ok(true)
    }
    ("y", 0) => {
        segment.op0(|| 20i32);
        Ok(true)
    }
    _ => Ok(false),
});

let mut segment = CELParser::new(lookup).parse_str("x + y").unwrap();
assert_eq!(segment.call0::<i32>().unwrap(), 30);
```

The parser tests for custom name lookup live in
[cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs).

## Notes

- Literal syntax is determined by Rust tokenization, not by a hand-written
  CEL-specific lexer.
- The book does not claim support for every CEL dialect literal form.
- See the parser grammar in `cel-parser/src/lib.rs` for the exact production
  rules that turn these tokens into expressions.
