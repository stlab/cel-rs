# Operators

CEL operators combine existing expression values. Precedence and
associativity determine how unparenthesized expressions group. For the
full grammar, see the [Reference Manual](reference.md).

## Concept

Unary, binary, cast, and postfix operators all participate in one
expression grammar. Postfix calls and tuple indices bind most tightly.
Range forms bind most loosely. Logical operators short-circuit, and
comparison and range forms do not chain.

## Syntax

```text
!x
-x
x as Type
a * b
a / b
a % b
a + b
a - b
a << b
a >> b
a & b
a ^ b
a | b
a == b
a != b
a < b
a <= b
a > b
a >= b
a && b
a || b
a..b
a..=b
a..
..b
..=b
..
```

## Worked examples

```text
10 + 20 * 5
"cel" + "-" + "lang"
ok && round(3.5) > 3.0
1u64 << 3u32
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
13. postfix calls and tuple indexing

Within one precedence level, binary operators associate to the left.
Comparison expressions consume one comparison operator, and range forms do
not chain.

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
- Range operators are documented in [Control flow](control-flow.md) and
  summarized in the [Reference Manual](reference.md).

## Edge cases

- Write `a < b && b < c`, not `a < b < c`.
- Write one range form at a time; `1..2..3` is not valid.
- Negative shift counts, counts larger than `u32`, and counts outside the
  left operand's width report `shift overflow`.
- Signed `+`, signed binary `-`, signed unary `-`, and signed `*`
  overflows report `arithmetic overflow`.
- Integer `/` and `%` report `division by zero` when they fail.
