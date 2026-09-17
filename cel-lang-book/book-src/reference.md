# Reference Manual

This chapter is the compact, reader-facing companion to `cel-parser/src/lib.rs`'s inline
grammar documentation. It keeps the supported CEL surface in one place and explains how the
parser and runtime model the surface.

## Operators and precedence

The expression grammar below shows how CEL precedence works from the top down: ranges bind
loosely, then boolean operators, then comparison and bitwise operators, then arithmetic, casts,
unary operators, postfix calls and tuple indexing, and finally the atomic primary expressions.

```text
expression = range_expression.
range_expression = or_expression [ ".." [ or_expression ] | "..=" or_expression ]
                  | ".." [ or_expression ]
                  | "..=" or_expression.
or_expression = and_expression { "||" and_expression }.
and_expression = comparison_expression { "&&" comparison_expression }.
comparison_expression = bitwise_or_expression
    [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.
bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.
bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.
bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.
additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.
multiplicative_expression = cast_expression { ("*" | "/" | "%") cast_expression }.
cast_expression = unary_expression { "as" identifier }.
unary_expression = (("-" | "!") unary_expression) | postfix_expression.
postfix_expression = primary_expression { "(" [ parameter_list ] ")" | "." unsuffixed_integer }.
parameter_list = expression { "," expression }.
```

Everything in this block is syntax: it tells you what parses, not how evaluation is
implemented. The actual operator behavior comes from `cel-runtime` and the operation lookup
tables described below.

## Grouping, tuples, arrays, and conditionals

Primary expressions are the atomic forms that can appear anywhere CEL expects a value. This
group includes literal values, identifiers, grouped subexpressions, tuple literals, array
literals, `if` expressions, and closures.

```text
primary_expression = literal | identifier | tuple_or_group | array_expression
                   | if_expression | closure_expression.
tuple_or_group = "(" [ expression ["," [ expression { "," expression } ]] ] ")".
array_expression = "[" expression { "," expression } "]".
if_expression = "if" expression "{" expression "}" [ "else" ( "{" expression "}" | if_expression ) ].
```

The difference between grouping and tuple syntax is part of CEL syntax, but the runtime
representation of tuples is implementation-specific: `cel-runtime` treats them as stack-shaped
values so later operations can recover element types and layout.

Arrays are also primary expressions, but the parser only accepts recursively homogeneous
arrays. A nested array literal like `[[0], [1]]` is therefore a recursively typed array of
arrays, not a mixed bag of unrelated values.

## Closures and parameter lists

Closures are CEL expressions whose body is another CEL expression. The parameter productions
below describe the syntax for the `|x, y| expr` form and for typed closure parameters.

```text
closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression.
closure_param = identifier ":" closure_type_expression.
closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")".
```

Closure parameter annotations currently accept the built-in scalar names and recursive tuple
syntax recognized by the parser. This type resolution is implementation-specific and is not
extended by a runtime type registry: the grammar above describes the syntax, while the parser's
known built-in names determine which annotations are accepted.

## Literal patterns

`literal_pattern` is a separate entry point used by grammars that embed CEL literals in
pattern position. It accepts a bare literal, or that literal preceded by a single `-`.

```text
literal_pattern = ["-"] literal.
```

This is intentionally narrower than full CEL unary syntax. It mirrors Rust's literal-pattern
rules for embedding use cases, rather than allowing every expression form.

## Type and operation model

`cel-parser` builds a typed runtime segment, not an AST that is evaluated later in a separate
pass. The parser checks enough type information to construct the runtime values it needs:
scalars become concrete runtime values, tuples become stack-layout aggregates, closures carry
their typed parameter descriptions, and arrays become recursively homogeneous
[`DynamicArray`](../cel_runtime/dynamic_array/struct.DynamicArray.html) values.

Operation lookup is also runtime-specific. `OpLookup` searches custom scopes in last-in,
first-out order, then falls back to the built-in operators and casts shipped with the parser.
Custom scopes are therefore a per-lookup overlay; built-ins remain the final fallback. When an
operation is selected, it is matched by operation name, arity, and operand `TypeId`s. That
matching model is implementation-specific, but the names and operators themselves are the CEL
syntax that users write.

The parser-time type checks are what make errors like mixed array element types and invalid
casts surface early. In other words: the grammar says whether an expression is legal CEL;
`cel-parser` and `cel-runtime` decide whether the expression can be given a concrete runtime
shape in this implementation.

## Known limitations

- Empty array literals (`[]`) are currently rejected because there is no element type to infer
  (<https://github.com/stlab/cel-rs/issues/212>).
- Heterogeneous arrays are rejected; every element must match the array's recursive element
  type.
- A tuple cannot be used as an array element in this implementation, so a literal such as
  `[(0i32, 1i32)]` is rejected (<https://github.com/stlab/cel-rs/issues/213>).
- Parser diagnostics and the static `ty::check_expr` checker use different span strategies for
  array element mismatches. The parser path underlines the full array literal while naming the
  offending element in the message; the static checker can point at the element's own span
  (<https://github.com/stlab/cel-rs/issues/215>).

## Synchronization guidance

The grammar in this chapter is copied from `cel-parser/src/lib.rs` and must stay in lockstep
with that module's rustdoc. Any grammar change must update both files in the same commit so the
inline API docs and the standalone book never drift apart.

When you touch parser grammar, update the reference manual, the parser module doc comment, and
any prose in the Adam book that links to CEL syntax at the same time.
