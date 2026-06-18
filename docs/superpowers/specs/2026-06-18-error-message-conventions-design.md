# Error Message Conventions — Design Spec

**Date:** 2026-06-18  
**Status:** Approved

## Problem

Error messages in cel-rs are inconsistent in capitalization, quoting style, and use of wrapper prefixes:

- `"Undefined identifier: {}"` — capital U
- `"operation error: Operation '{}' not found for types [{}]"` — redundant `operation error:` prefix wrapping a message that begins with a capital `Operation`
- `"operation error: {}"` — strips caller error context behind a redundant prefix
- `"lex: {}"` — terse, non-descriptive prefix wrapping a fixed string

Other messages (`"arithmetic overflow"`, `"division by zero"`, `"expected closing parenthesis"`, etc.) already follow the correct conventions.

## Rules

These rules apply to all error messages produced by this crate:

1. **Lowercase first letter.** Messages are always presented after an `"error: "` prefix in output. Starting lowercase matches the Rust stdlib, clippy, and thiserror convention.
2. **No trailing period.**
3. **Backticks around code tokens.** Identifiers, names, and type names are wrapped in backticks. Consistent with the literal-parsing errors already in the codebase (e.g. `` "invalid i32 literal `{value}`" ``).
4. **Passthrough errors surface verbatim.** No added wrapper prefix. Callers who supply errors — user-defined scope functions, external crates — are responsible for their own messages.
5. **"no X" phrasing for not-found errors.** Follows std convention (`"no method named"`, `"no field named"`).

## Changes

### `cel-runtime/src/parser/op_table.rs`

| Location | Before | After |
|---|---|---|
| lines 884, 897 | `format!("operation error: {}", e)` | `e.to_string()` |
| line 906 | `format!("Undefined identifier: {}", name)` | `` format!("undefined identifier: `{name}`") `` |
| lines 919–925 | `format!("operation error: Operation '{}' not found for types [{}]", name, type_names)` | `` format!("no operation `{name}` for types [{type_names}]") `` |

For the "no operation" case the type-name loop must also wrap each name in backticks:
```rust
type_names.push_str(&format!("`{}`", info.type_name.as_ref()));
```
so the result reads `` no operation `+` for types [`i32`, `u32`] ``.

### `cel-runtime/src/parser/mod.rs`

| Location | Before | After |
|---|---|---|
| line 342 | `format!("lex: {}", e)` | `e.to_string()` |

**Rationale:** `proc_macro2::LexError` always displays as `"cannot parse string into token stream"` (verified against proc-macro2 1.0.106 source). The message is already lowercase with no trailing period; going verbatim drops the uninformative `"lex: "` prefix. The attached span already locates the error.

### `ScopeFn` and `push_scope` doc comments

Add a convention note that error messages returned by scope functions must be lowercase, end without a period, and wrap identifiers in backticks — because they surface verbatim to the user.

## Scope

Only the four call sites above change. All other error messages already follow the rules and are left untouched.

## Verification

After changes, run:

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

No new tests are required; the changed messages are covered by existing tests that check for error conditions, and the updated strings can be verified by updating those test assertions.
