# Reference Manual

This chapter is the compact language reference for CEL expressions. It
gathers the core grammar, token categories, type names, literal forms,
operators, standard-library operations, collections, closures, and sharp edges in one
place.

## Concept

CEL is an expression language. Every source form in this chapter produces
a value or helps compose one. Use the tutorial chapters for worked prose,
and use this chapter when you need the exact grammar or a concise rule
table.

## Syntax

CEL uses strict LL(1) parsing with one token of lookahead. A comma requires
another item, except for the required terminal comma in a singleton tuple value
or type. No other list accepts a trailing comma.

```text
expression = ".." [ ranged_operand ] | "..=" ranged_operand | operand .
operand = ("-" | "!") operand | closure_expression | atom tail .
compared_operand = ("-" | "!") compared_operand | closure_expression | atom compared_tail .
ranged_operand = ("-" | "!") ranged_operand | closure_expression | atom ranged_tail .
ranged_compared_operand = ("-" | "!") ranged_compared_operand
                       | closure_expression | atom ranged_compared_tail .
atom = primary_expression { postfix_suffix } { "as" identifier } .
postfix_suffix = "(" [ argument_list ] ")" | "." (unsuffixed_integer | identifier) .
tail = [ high_operator operand | comparison_operator compared_operand
       | logical_operator operand | ".." [ ranged_operand ] | "..=" ranged_operand ] .
compared_tail = [ high_operator compared_operand | logical_operator operand
                | ".." [ ranged_operand ] | "..=" ranged_operand ] .
ranged_tail = [ high_operator ranged_operand | comparison_operator ranged_compared_operand
              | logical_operator ranged_operand ] .
ranged_compared_tail = [ high_operator ranged_compared_operand | logical_operator ranged_operand ] .
high_operator = "+" | "-" | "*" | "/" | "%" | "|" | "^" | "&" | "<<" | ">>" .
comparison_operator = "==" | "!=" | "<" | ">" | "<=" | ">=" .
logical_operator = "&&" | "||" .
argument_list = expression { "," expression } .
primary_expression = literal
                   | value_identifier [ record_fields ]
                   | tuple_or_group
                   | array_expression
                   | if_expression .
record_fields = "{"
    [ field_initializer { "," field_initializer } ] "}" .
field_initializer = identifier ":" expression .
tuple_or_group = "(" [ expression [ "," [ expression { "," expression } ] ] ] ")" .
array_expression = "[" [ expression { "," expression } ] "]"
                 [ ":" type_expr ] .
type_expr = identifier [ "(" [ type_expr { "," type_expr } ] ")" ]
          | "[" type_expr "]"
          | "(" [ type_expr [ "," [ type_expr { "," type_expr } ] ] ] ")" .
if_expression = "if" expression "{" expression "}"
              [ "else" ( "{" expression "}" | if_expression ) ] .
closure_expression = "||" expression
                   | "|" closure_param { "," closure_param } "|" expression .
closure_param = identifier ":" type_expr .
```

This grammar recognizes token structure. The precedence table below determines
operator binding, independently of the recognition grammar's right recursion.
Unary operators bind before casts; a closure body consumes the remaining expression
inside its delimiter boundary. The four operand/tail states reject chained ranges
and comparisons while allowing separate comparisons across logical operators.
A call suffix directly after a named member dispatches that member. Parenthesize
the member before the call suffix to call its result instead.

Expand the productions in ordinary and header modes. Header mode removes
`record_fields` from its primary production, reserving the following `{` for the
body. A condition enters header mode; a closure body inherits its enclosing mode.
Groups, arguments, arrays, record field values, and branch bodies enter ordinary
mode. `value_identifier` excludes `if`, `else`, `as`, `true`, and `false`.

Every operand chooses among unary punctuation, closure punctuation, and a primary
token. Every tail chooses among disjoint operator tokens or ends at a closing
delimiter, comma, body brace, or end of input. The optional ranged operand has
the same disjoint start and end sets. A consumed comma always requires another
item, except the required comma marking a singleton tuple.

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
| Built-in scalar types | `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64`, `bool`, `String` | These are the named cast targets. Closure annotations also accept registered cloneable leaves and built-in generic types through the configured type resolver. |
| Other value forms | `char`, byte strings, C strings, unit, tuples, arrays, ranges, closures | Expressions can use these values, but they are not extra built-in scalar type names. |
| Structural types | `()`, `(T,)`, `(T, U)`, `[T]`, `Fn((A, B), R)` | Compose recursively without registering each aggregate. `(T)` groups a type. `Fn((), R)` has no parameters; `Fn((T,), R)` has one. |

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
| 13 | calls, `.N`, `.name`, `.name(args)` | left |

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
| `as` | expression plus built-in scalar target name | Integer targets accept integer, floating-point, and `bool` sources. `f32`/`f64` targets accept integer and floating-point sources. `bool` and `String` targets accept only their same-type sources. |
| call and `.N` | a valid callee and arguments, or a tuple plus an unsuffixed integer | `.N` applies to tuples only. |

## Numeric call table

All functions in this table belong to the standard library and are available
when the evaluation environment installs it. See
[Standard library](standard-library.md) for worked examples and fuller prose.

| Function | Accepted operands | Result | Notes |
| --- | --- | --- | --- |
| `min(a, b)`, `max(a, b)` | same-type numeric operands: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64` | same as operands | No coercion. Successful calls are infallible. |
| `clamp(x, lo, hi)` | same-type numeric operands from the same 14-type set | same as operands | Requires ordered bounds with `lo <= hi`; otherwise reports `invalid clamp bounds`. |
| `abs(x)` | `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64` | same as operand | Minimum signed integer reports `arithmetic overflow`. |
| `signum(x)` | `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64` | same as operand | No unsigned support. Successful calls are infallible. |
| `sqrt(x)` | `f32`, `f64` | same as operand | Float-only. Inputs below zero yield `NaN` rather than an error. |
| `floor(x)`, `ceil(x)`, `trunc(x)`, `fract(x)`, `round_ties_even(x)` | `f32`, `f64` | same as operand | Float-only. Successful calls are infallible. |

- None of the documented functions are integer-only.
- `sqrt`, `floor`, `ceil`, `trunc`, `fract`, and `round_ties_even` are
  float-only.
- `min`, `max`, and `clamp` support both integers and floats.
- `abs` and `signum` support signed integers and floats, but not unsigned
  integers.

## Collection table

| Form | Syntax | Rules |
| --- | --- | --- |
| Unit | `()` | The empty parenthesized form is the unit value. |
| Grouping | `(expr)` | Changes grouping without creating a tuple. |
| Tuple | `(expr,)`, `(a, b, ...)` | Positional value form. A 1-tuple requires the trailing comma. |
| Array | `[a, b, ...]` | Unannotated arrays are non-empty, homogeneous, and comma-separated. |
| Typed array | `[a, b]: [T]`, `[]: [T]` | The annotation states the complete array type and permits empty arrays. |
| Nested array | `[[0], [1]]`, `[]: [[i32]]` | Each element must still have the same array element type. |
| Tuple array | `[(0, true)]`, `[]: [(i32, bool)]` | Each positional type participates in homogeneity. |
| Callable array | `[]: [Fn((i32,), [i32])]` | Empty arrays retain full signatures; equality is not required for storage. |
| Range | `a..b`, `a..=b`, `a..`, `..b`, `..=b`, `..` | Endpoint-bearing forms require homogeneous numeric endpoints. |

## Closure table

| Form | Meaning | Notes |
| --- | --- | --- |
| <code>&#124;&#124; expr</code> | zero-parameter closure | The body is one expression. |
| <code>&#124;x: T&#124; expr</code> | one typed parameter | `T` resolves through the configured type environment and may recursively contain tuples or arrays. |
| <code>&#124;x: T, y: U&#124; expr</code> | multiple typed parameters | Parameters are comma-separated and ordered left to right. |
| <code>&#124;pair: (i32, i32)&#124; expr</code> | one tuple-typed parameter | Tuple parameters are still one parameter and use `.N` inside the body. |
| nested tuple type | <code>&#124;value: (i32, (f64, bool))&#124; expr</code> | Tuple types may nest recursively. |
| <code>(&#124;x: f64&#124; x)(1.0)</code> | immediate invocation | CEL checks exact parameter/result shapes; arguments evaluate once, left to right. Unknown callable signatures fail explicitly. Host call operations retain precedence. |

## Exact rules

- Calls accept zero or more comma-separated arguments and do not accept a
  trailing comma.
- Tuple indexing uses `.N` with an unsuffixed integer and may be chained.
- Unannotated arrays are non-empty and homogeneous. A complete type
  annotation permits an empty array and must match the element type exactly.
- Tuples and arrays are distinct forms and compose recursively, including
  registered leaves, source unit, and callable values.
- Rust interoperability uses `Array<T>`, source-ordered canonical lists
  (`Nil`/`Cons` witnesses), and `Callable<Args, R>` (`FnOf` witness).
  `Array<Array<T>>` converts to `Vec<Array<T>>`, not allocation-preserving
  deep native vectors. See [collections](collections.md#rust-interoperability).
- Arbitrary erased external values require complete shape checks.
  `PreparedSegment` and `prepared_segment::PreparedRawSegment` validate at
  preparation, not each invocation. Clone, equality, and formatting are
  explicit independent capabilities.
- Native Rust operations can serve interpreted CEL through typed adapters.
  A macro-compiled CEL backend does not yet exist.
- Comparison expressions consume one comparison operator.
- Range forms do not chain.
- `expr.name` and `expr.name()` invoke the same host-provided unary operation.
  `expr.name(a, b)` passes the receiver first, followed by explicit arguments.
  Each operand evaluates once in source order. A value binding named `name`
  does not hide a dot operation, and the member label is not a variable reference.
- `(expr.name)()` calls the operation's result normally; it fails if the result
  is not callable. Dot operations, ordinary calls, and tuple indexing share
  postfix precedence and may be chained.
- `TypeName { field: expression, ... }` constructs a host-registered record.
  Constructor names belong to the type namespace, separate from value names.
  Field labels are not variable references. Every schema field is required exactly
  once; unknown, duplicate, missing, and incorrectly typed fields are errors.
  Field expressions evaluate once in written order, independent of schema order.
  An empty schema permits `TypeName {}`. Field lists do not permit trailing commas.
- Field types match exactly: an `f64` field accepts `1.0`, not the `i32` literal `1`.
  The static checker uses installed record schemas and operation signatures; known
  custom types retain their identities instead of unifying with unrelated types.
  Unknown constructors are errors. Unresolved inputs and calls without metadata
  remain best-effort, but all their child expressions are still checked.
- Unparenthesized record constructors are forbidden throughout an `if` condition.
  Parentheses, call arguments, tuple elements, and array elements start unrestricted
  expression positions: write `if (Sample { value: 3.0 }.value) > 0.0 { ... }`.
  The restriction propagates through operators, ranges, and closure bodies.
- Members are reads/operations, not assignment targets. CEL has no member assignment.
- CEL does not use bracket postfix indexing such as `expr[0]`.
- CEL does not define map literals, object literals, or a ternary
  operator.
- A closure body resolves its own parameters and does not capture free
  variables from the enclosing expression.
- A one-element tuple type requires `(T,)`; `(T)` is grouping.

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
- `[(0, 1), (true, 2)]` is not valid because element shapes differ.
