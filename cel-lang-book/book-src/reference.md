# Reference Manual

This chapter is the compact language reference for CEL expressions. It
gathers the core grammar, token categories, type names, literal forms,
operators, collections, closures, and sharp edges in one place.

## Concept

CEL is an expression language. Every source form in this chapter produces
a value or helps compose one. Use the tutorial chapters for worked prose,
and use this chapter when you need the exact grammar or a concise rule
table.

## Syntax

```text
expression = range_expression .
range_expression = ".." [ or_expression ]
                 | "..=" or_expression
                 | or_expression [ ".." [ or_expression ] | "..=" or_expression ] .
or_expression = and_expression { "||" and_expression } .
and_expression = comparison_expression { "&&" comparison_expression } .
comparison_expression = bitwise_or_expression
    [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ] .
bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression } .
bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression } .
bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression } .
bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression } .
additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression } .
multiplicative_expression = cast_expression { ("*" | "/" | "%") cast_expression } .
cast_expression = unary_expression { "as" identifier } .
unary_expression = (("-" | "!") unary_expression) | postfix_expression .
postfix_expression = primary_expression { "(" [ argument_list ] ")" | "." unsuffixed_integer } .
argument_list = expression { "," expression } .
primary_expression = literal
                   | identifier
                   | tuple_or_group
                   | array_expression
                   | if_expression
                   | closure_expression .
tuple_or_group = "(" [ expression [ "," [ expression { "," expression } ] ] ] ")" .
array_expression = "[" expression { "," expression } "]" .
if_expression = "if" expression "{" expression "}"
              [ "else" ( "{" expression "}" | if_expression ) ] .
closure_expression = "||" expression
                   | "|" closure_param { "," closure_param } "|" expression .
closure_param = identifier ":" closure_type_expression .
closure_type_expression = identifier
                        | "(" [ closure_type_expression { "," closure_type_expression } ] ")" .
```

## Worked examples

```text
(1, 2).0
if ok && ready { [1, 2] } else { [3, 4] }
|pair: (i32, i32)| pair.0 + pair.1
1 + 2..=3 * 4
true as i32
```

## Token table

| Category | Forms | Notes |
| --- | --- | --- |
| Identifiers | `name`, `value2`, `user_name` | Bare names for values, callees, and closure parameters. |
| Reserved words | `if`, `else`, `as`, `true`, `false` | These spellings have fixed language roles. |
| Literal tokens | integers, floats, strings, characters, bytes, byte strings, C strings | Unit `()` comes from grammar, not from a standalone literal token. |
| Delimiters | `(` `)` `[` `]` `{` `}` | Used for grouping, tuples, calls, arrays, and `if` branches. |
| Structural punctuation | `,` `:` `.` | Used for separators, closure parameter types, and tuple indexing. |
| Operators | <code>+</code> <code>-</code> <code>*</code> <code>/</code> <code>%</code> <code>!</code> <code>&amp;&amp;</code> <code>&#124;&#124;</code> <code>==</code> <code>!=</code> <code>&lt;</code> <code>&lt;=</code> <code>&gt;</code> <code>&gt;=</code> <code>&amp;</code> <code>&#124;</code> <code>^</code> <code>&lt;&lt;</code> <code>&gt;&gt;</code> <code>..</code> <code>..=</code> <code>as</code> | See the operator tables below. |

## Type table

| Kind | Names or forms | Notes |
| --- | --- | --- |
| Built-in scalar types | `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64`, `bool`, `String` | These are the only named cast targets and the only scalar names allowed in typed closure parameters. |
| Other value forms | `char`, byte strings, C strings, unit, tuples, arrays, ranges, closures | These values are accepted in expressions, but they are not extra built-in scalar type names. |

## Literal table

| Form | Example | Result |
| --- | --- | --- |
| Integer | `42`, `42u16` | Unsuffixed integers default to `i32`. Integer suffixes are `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, and `usize`. |
| Floating-point | `3.5`, `6.02e23f32` | Unsuffixed floating-point literals default to `f64`. Floating-point suffixes are `f32` and `f64`. |
| Boolean | `true`, `false` | Produces `bool`. |
| String | `"hello"` | Produces `String`. |
| Character | `'a'` | Produces `char`. |
| Byte | `b'A'` | Produces `u8`. |
| Byte string | `b"bytes"` | Produces a byte-string value. |
| C string | `c"header"` | Produces a C-string value. |
| Unit | `()` | Produces the unit value. |

## Operator precedence table

| Level (low to high) | Forms | Associativity |
| --- | --- | --- |
| 1 | range forms | non-chaining |
| 2 | <code>&#124;&#124;</code> | left |
| 3 | `&&` | left |
| 4 | `==`, `!=`, `<`, `<=`, `>`, `>=` | non-chaining |
| 5 | <code>&#124;</code> | left |
| 6 | `^` | left |
| 7 | <code>&amp;</code> | left |
| 8 | <code>&lt;&lt;</code>, <code>&gt;&gt;</code> | left |
| 9 | `+`, `-` | left |
| 10 | `*`, `/`, `%` | left |
| 11 | `as` | left |
| 12 | unary <code>-</code>, <code>!</code> | prefix |
| 13 | calls and `.N` | left |

## Operator rules table

| Forms | Accepted operands | Notes |
| --- | --- | --- |
| `+` | homogeneous numeric operands, or `String` + `String` | Unsigned integers wrap. Signed integers report `arithmetic overflow` on overflow. Strings concatenate. |
| binary `-` | homogeneous numeric operands | Unsigned integers wrap. Signed integers report `arithmetic overflow` on overflow. |
| unary `-` | signed integers and floating-point values | Signed integer overflow reports `arithmetic overflow`. |
| `*` | homogeneous numeric operands | Unsigned integers wrap. Signed integers report `arithmetic overflow` on overflow. |
| `/`, `%` | homogeneous numeric operands | Integer failures report `division by zero`. |
| <code>&amp;</code>, <code>&#124;</code>, <code>^</code> | homogeneous integer operands | Integer-only bitwise operators. |
| <code>&lt;&lt;</code>, <code>&gt;&gt;</code> | integer left operand and integer right operand | The right operand must fit `u32`, and the shift count must be in range. Failures report `shift overflow`. |
| `!` | `bool` | Logical negation. |
| <code>&amp;&amp;</code>, <code>&#124;&#124;</code> | `bool` and `bool` | Short-circuit logical operators. |
| `==`, `!=` | homogeneous numeric operands, homogeneous `bool`, or homogeneous `String` | Produces `bool`. |
| `<`, `<=`, `>`, `>=` | homogeneous numeric operands or homogeneous `String` | Produces `bool`. |
| range forms | homogeneous numeric endpoints, or no endpoints for `..` | `..=` requires a right endpoint. Endpoints are full expressions. |
| `as` | expression plus built-in scalar target name | Integer targets accept integer, floating-point, and `bool` sources. `f32`/`f64` targets accept integer and floating-point sources. `bool` and `String` only cast to themselves. |
| call and `.N` | a valid callee and arguments, or a tuple plus an unsuffixed integer | `.N` applies to tuples only. |

## Collection table

| Form | Syntax | Rules |
| --- | --- | --- |
| Unit | `()` | The empty parenthesized form is the unit value. |
| Grouping | `(expr)` | Changes grouping without creating a tuple. |
| Tuple | `(expr,)`, `(a, b, ...)` | Positional value form. A 1-tuple requires the trailing comma. |
| Array | `[a, b, ...]` | Non-empty, homogeneous, and comma-separated. |
| Nested array | `[[0], [1]]` | Each element must still have the same array element type. |
| Range | `a..b`, `a..=b`, `a..`, `..b`, `..=b`, `..` | Endpoint-bearing forms require homogeneous numeric endpoints. |

## Closure table

| Form | Meaning | Notes |
| --- | --- | --- |
| <code>&#124;&#124; expr</code> | zero-parameter closure | The body is one expression. |
| <code>&#124;x: T&#124; expr</code> | one typed parameter | `T` is a built-in scalar type name or a tuple type. |
| <code>&#124;x: T, y: U&#124; expr</code> | multiple typed parameters | Parameters are comma-separated and ordered left to right. |
| <code>&#124;pair: (i32, i32)&#124; expr</code> | one tuple-typed parameter | Tuple parameters are still one parameter and use `.N` inside the body. |
| nested tuple type | <code>&#124;value: (i32, (f64, bool))&#124; expr</code> | Tuple types may nest recursively. |

## Exact rules

- Calls accept zero or more comma-separated arguments and do not accept a
  trailing comma.
- Tuple indexing uses `.N` with an unsuffixed integer and may be chained.
- Arrays are non-empty and homogeneous.
- Tuples and arrays are distinct forms, and tuple values are not valid
  array elements.
- Comparison expressions consume one comparison operator.
- Range forms do not chain.
- CEL does not use member-name access such as `expr.name`.
- CEL does not use bracket postfix indexing such as `expr[0]`.
- CEL does not define map literals, object literals, or a ternary
  operator.
- A closure body resolves its own parameters and does not capture free
  variables from the enclosing expression.
- A closure parameter tuple type does not have a 1-element form, so
  `(T,)` is not accepted there.

## Edge cases

- Signed `+`, signed binary `-`, signed unary `-`, and signed `*`
  overflow report `arithmetic overflow`.
- Integer `/` and `%` failures report `division by zero`.
- Negative, too-large, or non-`u32`-fitting shift counts report
  `shift overflow`.
- `a < b < c` is not valid; write `a < b && b < c`.
- `1..2..3` is not valid.
- `..=` is not valid without a right endpoint.
- `[]` is not valid.
- `[(0, 1)]` is not valid.
