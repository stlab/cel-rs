# Control flow

CEL `if` expressions are value-producing expressions:

```rust
use cel_parser::{CELParser, OpLookup};

let mut segment = CELParser::new(OpLookup::new())
    .parse_str("if false { 1i32 } else if true { 2i32 } else { 3i32 }")
    .unwrap();

assert_eq!(segment.call0::<i32>().unwrap(), 2);
```

The checked examples in
[cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs)
cover the basic branch selection case, and the parser tests cover `else if`
chains and omitted `else` branches. The parser source in
[cel-parser/src/lib.rs](https://github.com/stlab/cel-rs/blob/main/cel-parser/src/lib.rs)
contains the exact grammar.

Like every other CEL expression in this implementation, the condition and
both branches still depend on the active `OpLookup` to resolve whatever values
and operations they use.

## `if` / `else if` / `else`

The supported form is:

```text
if condition { then_branch } else if other_condition { other_branch } else { fallback }
```

The condition and each branch are ordinary expressions. Branch result types
must agree, because the parser/runtimes join the two branch fragments into one
expression result.

## Ranges

The book also treats ranges here because the parser's range production is part
of the same expression layer and is used heavily by control-flow examples.

- `a..b` is an exclusive range.
- `a..=b` is an inclusive range.
- `a..` is a range from a lower bound.
- `..b` is a range to an upper bound.
- `..=b` is an inclusive upper bound.
- `..` is the full range.

```rust
use cel_parser::{CELParser, OpLookup};

let mut seg = CELParser::new(OpLookup::new()).parse_str("1i32..=5i32").unwrap();
assert_eq!(
    seg.call0::<std::ops::RangeInclusive<i32>>().unwrap(),
    1i32..=5i32
);
```

The parser tests in
[cel-parser/src/lib.rs](https://github.com/stlab/cel-rs/blob/main/cel-parser/src/lib.rs)
also show that range endpoints are full expressions and that `..=` without a
right endpoint is rejected.
