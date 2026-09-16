# Operators

Operator precedence follows the parser grammar, from lowest to highest:

1. logical `||`
2. logical `&&`
3. comparison `== != < > <= >=`
4. bitwise `| ^ &`
5. shift `<< >>`
6. additive `+ -`
7. multiplicative `* / %`
8. casts with `as`
9. unary `- !`
10. postfix call and tuple index

```rust
use cel_parser::{CELParser, OpLookup};

let mut segment = CELParser::new(OpLookup::new())
    .parse_str("10u32 + 20u32 * 5u32")
    .unwrap();

assert_eq!(segment.call0::<u32>().unwrap(), 110);
```

The checked examples in
[cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs)
cover arithmetic precedence, boolean operators, and custom operator lookup.

## Built-in operations and lookup

The parser does not hard-code evaluation for every symbol. It asks the current
`OpLookup` whether a name can be resolved for the operand types on the stack.
That is why built-in operations, custom scopes, and zero-argument identifiers
all share the same resolution path.

The practical result is:

- built-in arithmetic and comparisons work when their operand types are
  supported;
- a custom scope can add new zero-argument values or new operator behavior;
- parsing succeeds only when the lookup can build the needed operation.

## Comparison and ranges

Comparison operators do not chain. Ranges are parsed as expressions with very
low precedence, so the parser treats `1i32 + 2i32..3i32 * 4i32` as a range
whose endpoints are full expressions.

```rust
use cel_parser::{CELParser, OpLookup};

let mut seg = CELParser::new(OpLookup::new())
    .parse_str("1i32 + 2i32..3i32 * 4i32")
    .unwrap();
assert_eq!(seg.call0::<std::ops::Range<i32>>().unwrap(), 3i32..12i32);
```

See the range tests in the checked parser suite for the exclusive and
inclusive forms.
