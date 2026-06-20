# Design: `if` Expression and Short-Circuit `&&`/`||`

**Date:** 2026-06-19  
**Branch:** `sean-parent/if-and-short-circuit`

## Summary

Add `if`/`else`/`else if` expression support to the CEL parser and fix `&&`/`||` to short-circuit. Both features use `DynSegment::new_fragment()` and `DynSegment::join2()` — the existing conditional-execution primitive — rather than any new runtime infrastructure.

## Approach

Context-swap in the parser. When a branch expression needs lazy evaluation, the parser:

1. Creates a fragment: `let mut fragment = self.context.new_fragment()`
2. Swaps it into place: `std::mem::swap(&mut self.context, &mut fragment)`
3. Parses the sub-expression (ops land in the fragment)
4. Swaps back: `std::mem::swap(&mut self.context, &mut fragment)`
5. Calls `self.context.join2(then_fragment, else_fragment)`

`join2` pops a `bool` from the main stack, executes one of the two fragments, and pushes the result. Both fragments must return exactly one value of the same type.

## Grammar

```
expression        = or_expression
or_expression     = and_expression { "||" and_expression }
and_expression    = comparison_expression { "&&" comparison_expression }
...
primary_expression = literal
                   | identifier
                   | "(" or_expression ")"
                   | if_expression

if_expression = "if" or_expression "{" or_expression "}" [ else_clause ]
else_clause   = "else" ( "{" or_expression "}" | if_expression )
```

`if` is recognized as a keyword inside the `Identifier` arm of `is_primary_expression` (checked before `op_lookup` dispatch). `else` is consumed explicitly by the `if` handler and never reaches `op_lookup`.

## Short-Circuit `&&` and `||`

### `&&`

`is_and_expression` in `cel-parser/src/lib.rs` replaces the `op_lookup.lookup("&&", ...)` call with:

```
parse LHS → context stack: [bool]
rhs_fragment   = context.new_fragment(); parse RHS into it → [bool]
bypass_fragment = context.new_fragment(); bypass.just(false)
context.join2(rhs_fragment, bypass_fragment)
  // condition true  → execute rhs_fragment  (result = RHS value)
  // condition false → execute bypass_fragment (result = false, RHS skipped)
```

### `||`

`is_or_expression` replaces the `op_lookup.lookup("||", ...)` call with:

```
parse LHS → context stack: [bool]
rhs_fragment    = context.new_fragment(); parse RHS into it → [bool]
bypass_fragment = context.new_fragment(); bypass.just(true)
context.join2(bypass_fragment, rhs_fragment)
  // condition true  → execute bypass_fragment (result = true, RHS skipped)
  // condition false → execute rhs_fragment    (result = RHS value)
```

### Op-table cleanup

`LOGICAL_AND_SIGNATURES`, `LOGICAL_OR_SIGNATURES`, and their entries in the `OP_TABLE` phf map are removed from `cel-parser/src/op_table.rs`. Type checking is still enforced: `join2` verifies the stack top is `bool` and both fragments return the same type.

The `test_logical_and` test in `op_table.rs` is removed; coverage moves to parser-level tests.

## `if` Expression

`is_if_expression` is extracted as its own method and called from `is_primary_expression` when the identifier `"if"` is encountered.

### Parsing steps

1. Consume `"if"` identifier
2. Parse condition with `is_or_expression` — stops naturally at `{` (an `OpenDelim` matches no binary-op continuation)
3. Expect and consume `OpenDelim(Brace)`; error: `"expected '{' after if condition"`
4. Context-swap → parse then-branch with `is_or_expression` → swap back → `then_fragment`; error if empty: `"expected expression in then-branch"`
5. Expect and consume `CloseDelim(Brace)`; error: `"expected '}' after then-branch"`
6. Check for `Identifier("else")`:
   - **Present:** consume `"else"`, then:
     - `OpenDelim(Brace)` → context-swap → parse `is_or_expression` → swap back → `else_fragment`; consume `CloseDelim(Brace)`
     - `Identifier("if")` → call `is_if_expression` recursively; result becomes `else_fragment`
     - Anything else → error: `"expected '{' or 'if' after else"`
   - **Absent:** `else_fragment` = `context.new_fragment()` with `op0(|| ())`
7. `context.join2(then_fragment, else_fragment)`

### Optional `else`

When `else` is omitted, `else_fragment` pushes `()`. The `then_fragment` must also return `()` — `join2` enforces matching types and will error (`"fragment result types must match"`) if the then-branch produces a non-unit value.

### Type checking

`join2` already enforces:
- Condition on stack is `bool`
- Both fragments return exactly one value
- Both fragment result types match

No additional type-checking code is needed.

## Error Messages

| Situation | Message |
|-----------|---------|
| `&&`/`\|\|` RHS missing | `"expected comparison_expression"` / `"expected and_expression"` (unchanged) |
| `&&`/`\|\|` RHS wrong type | `"fragment result types must match"` (from `join2`) |
| Missing `{` after condition | `"expected '{' after if condition"` |
| Empty then-branch | `"expected expression in then-branch"` |
| Missing `}` after then-branch | `"expected '}' after then-branch"` |
| Missing `else` keyword when required | n/a — `else` is optional |
| Missing `{` or `if` after `else` | `"expected '{' or 'if' after else"` |
| Empty else-branch | `"expected expression in else-branch"` |
| Missing `}` after else-branch | `"expected '}' after else-branch"` |
| Branch type mismatch | `"fragment result types must match"` (from `join2`) |
| `if` without else, non-unit then | `"fragment result types must match"` (from `join2`) |

## Tests

All tests verify observable behavior from the contract (not implementation internals).

### Short-circuit tests (in `cel-parser/src/lib.rs`)

- `false && expr` returns `false` even when `expr` would error at runtime (short-circuit)
- `true || expr` returns `true` even when `expr` would error at runtime (short-circuit)
- `true && false` → `false`
- `false || true` → `true`
- Chained: `false && false && false` → `false`
- Chained: `true || false || false` → `true`
- Type error: `1i32 && true` → parse error (LHS not `bool`)

### `if` expression tests (in `cel-parser/src/lib.rs`)

- `if true { 1i32 } else { 2i32 }` → `1`
- `if false { 1i32 } else { 2i32 }` → `2`
- `if true { 1i32 } else if false { 2i32 } else { 3i32 }` → `1`
- `if false { 1i32 } else if false { 2i32 } else { 3i32 }` → `3`
- `if false { 1i32 } else if true { 2i32 } else { 3i32 }` → `2`
- `if true { () }` (omitted else) → `()`
- `if false { 1i32 }` (omitted else, type mismatch) → parse error
- `if true { 1i32 } else { true }` (branch type mismatch) → parse error
- `if true { 1i32 }` missing `{` → parse error with `"expected '{' after if condition"`
- `if true { 1i32 } else` trailing `else` → parse error

### Op-table tests

- `test_logical_and` in `op_table.rs` is removed; `&&`/`||` no longer dispatched through op-table

## Files Changed

| File | Change |
|------|--------|
| `cel-parser/src/lib.rs` | Rewrite `is_and_expression`, `is_or_expression`; add `is_if_expression`; extend `is_primary_expression` |
| `cel-parser/src/op_table.rs` | Remove `LOGICAL_AND_SIGNATURES`, `LOGICAL_OR_SIGNATURES`, their `OP_TABLE` entries, and `test_logical_and` |
