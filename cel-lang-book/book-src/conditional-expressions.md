# Conditional expressions

CEL uses value-producing `if` expressions to direct expression flow. For the
exact grammar, see the
[Reference Manual](reference.md).

## Concept

An `if` expression selects one branch value. It is an ordinary expression and
can appear anywhere an expression is expected.

## Syntax

```text
if_expression = "if" expression "{" expression "}"
              [ "else" ( "{" expression "}" | if_expression ) ] .
```

## Worked examples

```text
if ready { 1 } else { 0 }
if score > 90 { "high" } else if score > 75 { "mid" } else { "low" }
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

## Edge cases

- `if flag { () } else if other { () }` is valid without a final `else`
  because the omitted branch is also `()`.
- See the [Reference Manual](reference.md) for the complete branch grammar.
