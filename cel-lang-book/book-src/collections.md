# Collections

CEL uses tuples and arrays for compound values. Tuples preserve fixed
positional shape. Arrays hold homogeneous sequences of values; an empty array
uses a type annotation to state its element type. For the complete grammar,
see the [Reference Manual](reference.md).

## Concept

Parentheses and brackets build different kinds of values. Parentheses can
mean unit, grouping, or tuples. Brackets always mean arrays. That shape
difference matters because tuple indexing uses `.N`, while arrays follow
their own literal rules and do not use tuple indexing syntax.

## Syntax

```text
tuple_or_group = "(" [ expression [ "," [ expression { "," expression } ] ] ] ")" .
array_expression = "[" [ expression { "," expression } ] "]"
                 [ ":" type_expr ] .
type_expr = identifier
          | "[" type_expr "]"
          | "(" [ type_expr [ "," [ type_expr { "," type_expr } ] ] ] ")" .
```

## Worked examples

```text
(1,)
(1, 2, 3)
[0, 1, 2]
[[0], [1]]
[0, 1]: [i32]
[]: [i32]
[]: [[i32]]
if ready { [1, 2] } else { [3, 4] }
```

## Exact rules

- `()` is unit, `(expr)` is grouping, `(expr,)` is a 1-tuple, and
  `(a, b, c)` is a tuple.
- `[a, b, c]` is an array literal.
- An unannotated array is non-empty and every element must have the same type.
- A type annotation after `:` states the complete array type. It permits an
  empty array, as in `[]: [i32]`.
- A non-empty array annotation must match the element type exactly.
- Nested arrays are valid when each element is itself an array of the
  same element type.
- Tuples and arrays are distinct value forms: tuples are positional and
  use `.N`, while arrays use bracket literals and homogeneous element
  typing.
- Tuple indexing uses an unsuffixed integer such as `.0` or `.1`; member
  access such as `value.name` is not part of CEL.
- Tuple values are not valid array elements.

## Edge cases

- `[]` is not valid because an empty array needs a type annotation.
- `[0,]` is not valid.
- `[]: [i32]` is a valid typed empty array.
- `[0, true]` is not valid because the element types differ.
- `[(0, 1)]` is not valid because tuple values are not array elements.
- `[]: [[i32]]` is a valid typed empty nested array.
- Tuple-valued array element annotations are not supported.
- Bracket postfix indexing such as `items[0]` is not part of the
  language.
- A suffixed tuple index such as `.0i32` is not valid.
