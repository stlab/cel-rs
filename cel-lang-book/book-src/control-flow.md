# Control flow

CEL uses value-producing `if` expressions and first-class range values to
direct expression flow. For the exact grammar, see the
[Reference Manual](reference.md).

## Concept

An `if` expression selects one branch value. A range expression builds a
range value from zero, one, or two numeric endpoints. Both forms are
ordinary expressions and can appear anywhere an expression is expected.

## Syntax

```text
if condition { then_expression }
if condition { then_expression } else { else_expression }
if condition { then_expression } else if other_condition { other_expression } else { fallback_expression }

start..end
start..=end
start..
..end
..=end
..
```

## Worked examples

```text
if ready { 1 } else { 0 }
if score > 90 { "high" } else if score > 75 { "mid" } else { "low" }
0..limit
..=10
if open { 1..=5 } else { 10.. }
```

## Exact rules

- `if` is an expression, not a statement. It yields the value of the
  selected branch.
- The supported branch forms are `if`, `if ... else`, and `if ... else
  if ...` chains with an optional final `else`.
- The condition and every branch body are ordinary expressions.
- The braces belong to `if` syntax. CEL does not use free-standing block
  expressions.
- The supported range forms are `a..b`, `a..=b`, `a..`, `..b`, `..=b`,
  and `..`.
- Endpoint-bearing ranges require homogeneous numeric endpoints.
- Range endpoints are full expressions, so operators inside either side
  are parsed before the range is formed.
- Range expressions have the lowest precedence in the language.

## Edge cases

- `..=` always requires a right endpoint.
- `1 + 2..3 * 4` means `(1 + 2)..(3 * 4)`.
- `1..2..3` is not valid.
- `if flag { 1 } else if other { 2 }` is valid even without a final
  `else`.
- See the [Reference Manual](reference.md) for the complete branch and
  range grammar.
