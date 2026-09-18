# Conditional expressions

CEL uses value-producing `if` expressions and first-class range values to
direct expression flow. For the exact grammar, see the
[Reference Manual](reference.md).

## Concept

An `if` expression selects one branch value. A range expression builds a
range value from zero, one, or two numeric endpoints. Both forms are
ordinary expressions and can appear anywhere an expression is expected.

## Syntax

```text
if_expression = "if" expression "{" expression "}"
              [ "else" ( "{" expression "}" | if_expression ) ] .
range_expression = ".." [ or_expression ]
                 | "..=" or_expression
                 | or_expression [ ".." [ or_expression ] | "..=" or_expression ] .
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
- CEL supports the branch forms `if`, `if ... else`, and `if ... else if ...`
  chains.
- The condition and every branch body are ordinary expressions.
- Omitting the final `else` supplies an implicit `()` branch, so the
  remaining branches must still be type-compatible with unit.
- The braces belong to `if` syntax. CEL does not use free-standing block
  expressions.
- CEL supports the range forms `a..b`, `a..=b`, `a..`, `..b`, `..=b`, and
  `..`.
- Endpoint-bearing ranges require homogeneous numeric endpoints.
- Range endpoints are full expressions, so operators inside either side
  are parsed before the range is formed.
- Range expressions have the lowest precedence in the language.

## Edge cases

- `..=` always requires a right endpoint.
- `1 + 2..3 * 4` means `(1 + 2)..(3 * 4)`.
- `1..2..3` is not valid.
- `if flag { () } else if other { () }` is valid without a final `else`
  because the omitted branch is also `()`.
- See the [Reference Manual](reference.md) for the complete branch and
  range grammar.
