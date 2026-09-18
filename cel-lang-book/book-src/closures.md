# Closures

CEL uses `|...|` for closure literals. A closure is an expression and can
appear anywhere an expression is expected. For the exact grammar, see the
[Reference Manual](reference.md).

## Concept

A closure packages a parameter list and a single expression body into a
callable value.

## Syntax

```text
closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|")
                     expression .
closure_param = identifier ":" closure_type_expression .
closure_type_expression = identifier
                        | "(" [ closure_type_expression
                                { "," closure_type_expression } ] ")" .
```

## Worked examples

```text
|x: i32| x + 1
|pair: (i32, i32)| pair.0 + pair.1
|| 1..=5
```

## Exact rules

Closures:

- `|| body` declares a zero-parameter closure.
- `|name: Type| body` declares one typed parameter.
- `|a: T, b: U| body` declares multiple typed parameters in source order.
- A closure parameter type is either a built-in scalar name or a
  parenthesized tuple type built recursively from closure parameter
  types.
- `|pair: (i32, i32)| pair.0 + pair.1` declares one tuple-typed
  parameter, not two separate parameters.
- The closure body is one expression.
- A closure body resolves its own parameters and does not capture free
  variables from an enclosing expression.

## Edge cases

- Closure parameter lists do not accept a trailing comma.
- `(i32,)` is not accepted as a one-element closure tuple type.
- Closure bodies do not introduce statement blocks or local binding
  syntax.
