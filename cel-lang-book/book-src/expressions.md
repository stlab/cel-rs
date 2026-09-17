# Expressions

A CEL expression is the unit the parser consumes and the runtime executes.
`CELParser::parse_str` returns a segment, not a final value; the segment only
becomes a value when you call it.

```rust
use cel_parser::{CELParser, OpLookup};

let mut segment = CELParser::new(OpLookup::new()).parse_str("1i32 + 2i32").unwrap();
assert_eq!(segment.call0::<i32>().unwrap(), 3);
```

That same split shows up in the checked examples in
[cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs).

## Identifiers and call arguments

Identifiers can be used as zero-argument lookups or as values consumed by
operators and calls. A parameter list is just a comma-separated list of
expressions:

```rust
use cel_parser::{CELParser, OpLookup};

let mut lookup = OpLookup::new();
lookup.push_scope(|name, segment, num_operands, _span| match (name, num_operands) {
    ("f", 0) => {
        segment.op0(|| 0i32);
        Ok(true)
    }
    ("()", 2) => {
        segment.op2(|_callee: i32, arg: std::ops::Range<i32>| arg)?;
        Ok(true)
    }
    _ => Ok(false),
});

let mut segment = CELParser::new(lookup).parse_str("f(1i32..5i32)").unwrap();
assert_eq!(segment.call0::<std::ops::Range<i32>>().unwrap(), 1i32..5i32);
```

The parser tests also show identifiers resolved from custom scopes with
`x + y`.

## Grouping and tuples

Parentheses are overloaded on purpose:

- `()` parses as unit.
- `(expr)` groups one expression and returns that expression's value.
- `(expr,)` is a 1-tuple.
- `(expr, expr, ...)` is a tuple with multiple elements.

That difference matters because tuple syntax becomes a stack-shaped runtime
value, while grouping only changes precedence.

```rust
use cel_parser::{CELParser, OpLookup};

let mut segment = CELParser::new(OpLookup::new()).parse_str("(1i32 + 2i32)").unwrap();
assert_eq!(segment.call0::<i32>().unwrap(), 3);
```

The tuple/group split is exercised in the parser implementation and in the
checked examples.

## Scope of expressions

An expression can nest arrays, closures, conditionals, ranges, and operators.
The parser grammar in `cel-parser/src/lib.rs` is the authoritative source for
which syntactic forms are legal.
