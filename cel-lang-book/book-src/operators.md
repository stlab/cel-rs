# Operators

CEL operators combine existing expression values. Precedence and
associativity determine how unparenthesized expressions group. For the
full grammar, see the [Reference Manual](reference.md).

## Concept

Unary, binary, cast, and postfix operators all participate in one
expression grammar. Postfix calls, tuple indices, and named members bind most tightly.
Range forms bind most loosely. Logical operators short-circuit, and
comparison and range forms do not chain.

## Syntax

```text
expression = closure_expression | range_expression .
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
postfix_expression = primary_expression { postfix_suffix } .
postfix_suffix = "(" [ argument_list ] ")" | "." (unsuffixed_integer | identifier) .
argument_list = expression { "," expression } .
```

## Worked examples

```text
10 + 20 * 5
"cel" + "-" + "lang"
ok && round_ties_even(3.5) > 3.0
1u64 << 3u32
1.5 as i32
1 + 2..3 * 4
```

## Exact rules

Precedence runs from lowest to highest in this order:

1. range forms
2. `||`
3. `&&`
4. comparison operators
5. bitwise `|`
6. bitwise `^`
7. bitwise `&`
8. shifts `<<` and `>>`
9. additive `+` and `-`
10. multiplicative `*`, `/`, and `%`
11. casts with `as`
12. unary `-` and `!`
13. postfix calls, tuple indexing, and named members

Within one precedence level, binary operators associate to the left.
Comparison expressions consume one comparison operator, and range forms do
not chain.

- A call is a postfix `()` form with zero or more comma-separated arguments.
- `receiver.name` and `receiver.name()` both invoke a unary named operation,
  ignoring same-named value bindings. `receiver.name(a, b)` supplies the receiver
  first, then explicit arguments; each evaluates once in source order.
  The member label is not a variable reference, even inside a closure.
- Grouping changes call semantics: `(receiver.name)()` calls the accessor's
  result as a value. It is not another spelling of `receiver.name()`.
  Named members, calls, and `.N` share postfix precedence and chain left to right.
  Neither named members nor tuple indices support assignment.
- Hosts supply member signatures, not a fixed set of receiver types. The checker
  rejects known incompatible operands and preserves registered result types.
  Calls without metadata remain best-effort and still check every argument.
- A bare closure is a complete expression, not an operator operand. Group it
  before applying an operator or postfix suffix: `(|x: i32| x + 1)(2)`.
- `+` accepts homogeneous numeric operands and `String + String`.
  Unsigned integer addition wraps. Signed integer addition reports
  `arithmetic overflow` on overflow.
- Binary `-` accepts homogeneous numeric operands. Unary `-` accepts only
  signed integers and floating-point values. Unsigned integer subtraction
  wraps. Signed binary subtraction and signed unary negation report
  `arithmetic overflow` on overflow.
- `*` accepts homogeneous numeric operands. Unsigned integer
  multiplication wraps. Signed integer multiplication reports
  `arithmetic overflow` on overflow.
- `/` and `%` accept homogeneous numeric operands. Floating-point
  division and remainder use normal floating-point behavior. Integer
  failures report `division by zero`.
- `&`, `|`, and `^` accept homogeneous integer operands only.
- `<<` and `>>` accept integer operands only. The right operand must fit
  in `u32`, and the shift count must be in range. Shift failures report
  `shift overflow`.
- `!` accepts `bool` only.
- `&&` and `||` accept `bool` operands only and short-circuit.
- `==` and `!=` accept homogeneous numeric operands, homogeneous `bool`,
  or homogeneous `String`.
- `<`, `<=`, `>`, and `>=` accept homogeneous numeric operands or
  homogeneous `String`.
- The valid range forms
  are `a..b`, `a..=b`, `a..`, `..b`, `..=b`, and `..`.
- `..=` always requires a right endpoint.
- Range endpoints are `or_expression`s. Group a closure or nested range used
  as an endpoint; endpoint types must still satisfy the operator's requirements.
- Range expressions do not chain, so `1..2..3` is not valid.
- `1 + 2..3 * 4` means `(1 + 2)..(3 * 4)`.
- `as` converts a value to one of the sixteen built-in scalar type names:
  `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`,
  `u128`, `usize`, `f32`, `f64`, `bool`, and `String`.
- Integer targets accept integer, floating-point, and `bool` sources.
  Floating-point targets accept integer and floating-point sources.
  `bool` and `String` targets accept only their same-type sources.
- Integer-to-integer casts check that the source fits in the target.
  Floating-point-to-integer casts require a finite, in-range source and
  truncate toward zero. `f64 as f32` checks finite range before narrowing.
- `true` casts to integer `1`, and `false` casts to integer `0`.
- `char`, byte-string, C-string, unit, array, tuple, range, and closure
  values are not additional cast target names.
- Number-to-`bool`, `bool`-to-float, and `String`-to-number or
  `String`-to-`bool` casts are not valid.
- Casts associate from left to right, so `x as T as U` applies the first
  cast before the second.

## Edge cases

- Write `a < b && b < c`, not `a < b < c`.
- Call argument lists do not accept a trailing comma.
- Negative shift counts, counts larger than `u32`, and counts outside the
  left operand's width report `shift overflow`.
- Signed `+`, signed binary `-`, signed unary `-`, and signed `*`
  overflows report `arithmetic overflow`.
- Integer `/` and `%` report `division by zero` when they fail.
- Casts cannot target `char`, byte-string, C-string, unit, array, tuple,
  range, or closure values.
