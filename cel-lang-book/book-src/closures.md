# Closures

CEL uses `|...|` for closure literals. A bare closure occupies a complete
expression; an operator operand requires parentheses around the closure.
For the exact grammar, see the
[Reference Manual](reference.md).

## Concept

A closure packages a parameter list and a single expression body into a
callable value.

## Syntax

```text
expression = closure_expression | range_expression .
closure_expression = ("||" | "|" closure_param { "," closure_param } "|")
                     expression .
closure_param = identifier ":" type_expression .
type_expression = identifier [ "(" [ type_expression { "," type_expression } ] ")" ]
                | "[" type_expression "]"
                | "(" [ type_expression [ "," [ type_expression { "," type_expression } ] ] ] ")" .
```

## Worked examples

```text
|x: i32| x + 1
|pair: (i32, i32)| pair.0 + pair.1
|| 1..=5
(|x: f64| x)(1.0)
(|pair: (i32, i32)| pair.0 + pair.1)((2, 3))
(|rows: [[i32]]| rows)([[1], [2]])
|f: Fn((i32,), [i32])| f(42)
[]: [Fn((i32,), [i32])]
```

## Exact rules

Closures:

- `|| body` declares a zero-parameter closure; spaced empty pipes `| |` are not this token.
- `|name: Type| body` declares one typed parameter.
- `|a: T, b: U| body` declares multiple typed parameters in source order.
- An annotation uses the host's configured type environment: built-in
  scalars and generic types, registered cloneable leaves, recursive tuple
  types, recursive array types, and structural `Fn` signatures. Container
  shapes require no separate registration.
- `|pair: (i32, i32)| pair.0 + pair.1` declares one tuple-typed
  parameter, not two separate parameters.
- The closure body is one expression.
- A bare closure body extends to its enclosing expression boundary. For example,
  `|| 1 + 2` returns `3`; write `(|| 1)()` to apply a zero-parameter closure.
- Group a closure before using it as an operator operand, cast input, or postfix
  receiver. `a + |x: i32| x` is a syntax error; `a + (|x: i32| x)` is
  syntactically valid, with operator/type checking performed separately.
- Call arguments, tuple and array elements, branch bodies, and closure bodies
  are complete expressions and accept bare closures, as in `f(|x: i32| x + 1)`.
- Parameter names are unique and cannot use reserved value names.
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

## Typed callable interoperability

`Fn((A, B), R)` has two parameters, `Fn((A,), R)` has one, and `Fn((), R)`
has none. The parameter list is always a tuple type: `Fn(A, R)` does not
stand for one parameter. A parameter can itself be a tuple, array, or callable.
Arrays compare whole signatures when checking element homogeneity.

`Callable<Args, R>` uses concrete canonical list arguments and an owned result.
`FnOf<ArgsWitness, ResultWitness>` supplies its complete structural witness.
`Callable::from_fn` binds a native function pointer;
`Callable::from_prepared` binds a matching `PreparedSegment`. Both expose the
same `call(&args)` interface with no signature comparisons during invocation.
Clones share execution state; recursive entry into the same carrier returns an
error. Body errors propagate. This interface prepares for a future compiled
backend; that backend is not implemented.

This example checks a typed empty array and calls native and interpreted
functions through the same Rust interface. Callables need no equality capability
to be array elements:

```rust
{{#include ../tests/composable_containers.rs:callable-arrays}}
```

## Edge cases

- Closure parameter lists do not accept a trailing comma.
- `(i32,)` is a one-element tuple type; `(i32)` groups the scalar type.
- Closure bodies do not introduce statement blocks or local binding
  syntax.
