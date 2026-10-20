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
closure_param = identifier ":" type_expression .
type_expression = identifier [ "(" type_expression { "," type_expression } ")" ]
                | "[" type_expression "]"
                | "(" [ type_expression { "," type_expression } ] ")" .
```

## Worked examples

```text
|x: i32| x + 1
|pair: (i32, i32)| pair.0 + pair.1
|| 1..=5
(|x: f64| x)(1.0)
(|pair: (i32, i32)| pair.0 + pair.1)((2, 3))
(|rows: [[i32]]| rows)([[1], [2]])
```

## Exact rules

Closures:

- `|| body` declares a zero-parameter closure.
- `|name: Type| body` declares one typed parameter.
- `|a: T, b: U| body` declares multiple typed parameters in source order.
- An annotation uses the host's configured type environment: built-in
  scalars and generic types, registered cloneable leaves, recursive tuple
  types, and recursive array types. Arrays retain the existing restriction
  that their elements cannot be tuples.
- `|pair: (i32, i32)| pair.0 + pair.1` declares one tuple-typed
  parameter, not two separate parameters.
- The closure body is one expression.
- A closure body resolves its own parameters and does not capture free
  variables from an enclosing expression. Installed library operations
  remain available inside it.
- Parenthesize a closure to invoke it immediately. CEL checks the exact
  parameter and result shapes, including nested tuples and array element
  types. Arguments evaluate once, left to right; body errors propagate to
  the enclosing evaluation.
- Known signatures travel through nested calls and tuple intermediates.
  Conditional closure alternatives must have identical known signatures.
  An opaque host-provided closure with no known signature reports an
  unsupported-call error rather than guessing a result type. A host may
  still supply its own call operation, which takes precedence.

## Host registration

`CELParser::with_type_resolver` supplies annotation lookup. A registered
leaf uses `ValueType::cloneable_leaf::<T>()` so parameters can clone
owned values without requiring `Copy`, equality, or debug formatting.
Adam supplies these descriptors through its existing `TypeRegistry`;
no second closure-specific type registry is needed. Results move from
the body into the caller and need no clone adapter of their own.

The Rust `DynClosure::new`, `call`, and `call_boxed` APIs remain available
for existing host integrations. `DynClosure::typed` and
`DynSegment::push_closure` additionally retain the complete metadata
needed by general CEL invocation.

## Edge cases

- Closure parameter lists do not accept a trailing comma.
- `(i32,)` is not accepted as a one-element closure tuple type.
- Closure bodies do not introduce statement blocks or local binding
  syntax.
