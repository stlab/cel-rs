# Expressions

A CEL expression produces a value. Expressions can stand alone or nest
inside other expressions. For the complete grammar and the exact language
rules, see the [Reference Manual](reference.md).

## Concept

CEL builds larger expressions from smaller ones. Identifiers name values.
Parentheses group subexpressions or build tuples. Postfix syntax covers
calls and tuple indexing. Arrays, conditionals, ranges, casts, and
closures are all expressions, so they can appear anywhere an expression is
expected.

## Syntax

```text
identifier
expression(arguments)
expression.N
()
(expression)
(expression,)
(expression, expression, ...)
[expression, expression, ...]
if expression { expression } else { expression }
expression as Type
|| expression
|name: Type| expression
left .. right
```

## Worked examples

```text
value
round(3.5)
(1 + 2) * 3
(10, 20).1
if ready { [1, 2] } else { [3, 4] }
```

## Exact rules

- An identifier is a bare name used as a value, a callee, or a closure
  parameter.
- A call is a postfix `()` form with zero or more comma-separated
  arguments.
- Tuple indexing is a postfix `.N` form where `N` is an unsuffixed
  integer such as `0` or `1`.
- `()` is the unit value, `(expr)` is grouping, `(expr,)` is a 1-tuple,
  and `(a, b, c)` is a tuple.
- Arrays, `if` expressions, ranges, casts, and closures are ordinary
  expression forms and may nest inside calls, tuples, arrays, and one
  another.
- Postfix calls and tuple indices bind more tightly than unary and binary
  operators.

## Edge cases

- `.` introduces tuple indexing only. CEL does not use `expr.name`
  member access.
- Bracket indexing such as `items[0]` is not part of the language.
- Call argument lists do not accept a trailing comma.
- Tuple indices must be unsuffixed integers, so `.0i32` is not valid.
- See the [Reference Manual](reference.md) for the full expression
  grammar.
