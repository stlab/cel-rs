# Collections

CEL uses tuples and arrays for compound values. Tuples preserve fixed
positional shape. Arrays hold a non-empty homogeneous sequence of values.
For the complete grammar, see the [Reference Manual](reference.md).

## Concept

Parentheses and brackets build different kinds of values. Parentheses can
mean unit, grouping, or tuples. Brackets always mean arrays. That shape
difference matters because tuple indexing uses `.N`, while arrays follow
their own literal rules and do not use tuple indexing syntax.

## Syntax

```text
()
(expression)
(expression,)
(expression, expression, ...)
[expression, expression, ...]
[[expression], [expression]]
```

## Worked examples

```text
(1,)
(1, 2, 3)
[0, 1, 2]
[[0], [1]]
if ready { [1, 2] } else { [3, 4] }
```

## Exact rules

- `()` is unit, `(expr)` is grouping, `(expr,)` is a 1-tuple, and
  `(a, b, c)` is a tuple.
- `[a, b, c]` is an array literal.
- Arrays are non-empty.
- Every array element must have the same type.
- Nested arrays are valid when each element is itself an array of the
  same element type.
- Tuples and arrays are distinct value forms: tuples are positional and
  use `.N`, while arrays use bracket literals and homogeneous element
  typing.
- Tuple values are not valid array elements.

## Edge cases

- `[]` is not valid.
- `[0,]` is not valid.
- `[0, true]` is not valid because the element types differ.
- `[(0, 1)]` is not valid because tuple values are not array elements.
- Bracket postfix indexing such as `items[0]` is not part of the
  language.
