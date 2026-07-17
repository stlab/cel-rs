# pm-lang Design

**Date:** 2026-06-28
**Author:** Sean Parent
**Status:** Approved

> **Superseded in part:** the `output_list` grammar and "Multi-output tuple
> methods" section below describe pm-lang's original hand-rolled tuple splitting,
> written before `cel-parser` had native tuple support. See
> `docs/superpowers/specs/2026-07-17-pm-lang-native-tuple-outputs-design.md` for the
> current design.

## Overview

A new `pm-lang` crate implementing a DSL parser for property models. The language is expressed in Wirth EBNF, follows the same lexer conventions as `cel-parser`, and uses `CELParser` for method body expressions. Parsing a pm-lang source string produces a live `Sheet` from the `property-model` crate.

Related issues:

- [#29](https://github.com/stlab/cel-rs/issues/29) — equation-style relationship declarations (future)
- [#30](https://github.com/stlab/cel-rs/issues/30) — tuple expressions in CEL (future)

## Placement

New crate `pm-lang/` added to `[workspace] members` in the root `Cargo.toml`. No existing workspace crate depends on `pm-lang`.

## Grammar

Wirth EBNF; `or_expression` is the full CEL grammar from `cel-parser`.

```ebnf
sheet = "sheet" identifier "{" { sheet_item } "}".
sheet_item = cell_decl | relationship_decl | conditional_decl.

cell_decl      = "cell" identifier cell_type_init ";".
cell_type_init = ":" type_name "=" literal   (* annotation + initializer *)
               | ":" type_name               (* annotation, default-init *)
               | "=" literal.                (* initializer, type inferred *)

type_name = identifier.

relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".
method_decl = "method" cell_list "->" cell_list method_body ";".
cell_list   = "[" identifier { "," identifier } "]".
method_body = "{" output_list "}".
output_list = "(" or_expression "," or_expression { "," or_expression } ")"
            | or_expression.

conditional_decl   = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".
conditional_branch = literal "=>" "{" { method_decl } "}" [ "," ].
default_branch     = "_"     "=>" "{" { method_decl } "}" [ "," ].
```

### Grammar notes

- `cell_type_init`: annotation is optional when an initializer is provided; the initializer is
  optional when an annotation is provided. Both omitted is a parse error.
- Cell initializers are restricted to **literals** in this version. Full-expression initializers
  are future work.
- The tuple form `(e₁, e₂, …)` in `output_list` requires at least two expressions separated by
  commas. A single parenthesized expression is still an `or_expression`, not a tuple.
  Multi-output tuple bodies are handled entirely by pm-lang (each element compiles to an
  independent `DynSegment`); issue [#30](https://github.com/stlab/cel-rs/issues/30) covers
  native tuple support in CEL.
- `->` and `=>` are added to `LexLexer::is_compound_operator`; no other changes to the
  existing lexer behavior.

### Example

```text
sheet image_resize {
    cell width:  f64 = 1920.0;
    cell height: f64 = 1080.0;
    cell area:   f64;           // default-initialised (0.0)
    cell ratio:  f64 = 1.0;
    cell mode:   i32 = 0;

    relationship {
        method [width, height] -> [area]   { width * height }
        method [area, height]  -> [width]  { area / height }
        method [width, area]   -> [height] { area / width }
    }

    conditional mode {
        0i32 => {
            method [width] -> [height] { width }
        }
        1i32 => {
            method [width, ratio] -> [height] { width * ratio }
        }
        _ => {
            method [width] -> [height] { width }
        }
    }
}
```

## TypeRegistry API

```rust
/// Maps type-name strings in cell declarations to Rust types.
pub struct TypeRegistry { /* ... */ }

impl TypeRegistry {
    /// Creates a registry pre-populated with all built-in CEL/Rust primitive types.
    pub fn new() -> Self;

    /// Registers a type that supports default initialisation (allows `cell x: T;`).
    pub fn register<T: Any + PartialEq + Default + 'static>(&mut self, name: &str);

    /// Registers a type without a default (requires `= literal` initializer).
    pub fn register_no_default<T: Any + PartialEq + 'static>(&mut self, name: &str);
}
```

Built-ins registered by `TypeRegistry::new()`:

| DSL name | Rust type | Has default |
| -------- | --------- | ----------- |
| `i8`…`i128`, `isize` | `i8`…`i128`, `isize` | yes (0) |
| `u8`…`u128`, `usize` | `u8`…`u128`, `usize` | yes (0) |
| `f32`, `f64` | `f32`, `f64` | yes (0.0) |
| `bool` | `bool` | yes (false) |
| `String` | `String` | yes ("") |

`char` and byte/C-string types are not pre-registered; clients register them via
`register` or `register_no_default` if needed.

Resolving a `type_name` that is not in the registry is a parse-time error. Using a
default-less type in a cell with no initializer is also a parse-time error.

## PmParser API

```rust
/// Parser result type — reuses cel-parser's ParseError for consistent diagnostics.
pub type Result<T> = std::result::Result<T, ParseError>;

pub struct PmParser {
    types: TypeRegistry,
    op_lookup: OpLookup,
}

impl PmParser {
    /// Creates a parser with the given type registry and operation lookup.
    ///
    /// `op_lookup` is forwarded to the embedded `CELParser` when compiling
    /// method body expressions, giving callers control over custom operators
    /// and functions available inside method bodies.
    pub fn new(types: TypeRegistry, op_lookup: OpLookup) -> Self;

    /// Parses a pm-lang source string into a live [`Sheet`].
    ///
    /// # Errors
    ///
    /// Returns `Err` on any syntax error, unknown type name, type mismatch
    /// between a cell annotation and its initializer or method output, undeclared
    /// cell name in a method cell list, or arity mismatch between an `output_list`
    /// tuple and the method's declared outputs.
    pub fn parse_str(&mut self, source: &str) -> Result<Sheet>;
}
```

### Usage example

```rust
use pm_lang::{PmParser, TypeRegistry};
use cel_parser::op_table::OpLookup;

let mut parser = PmParser::new(TypeRegistry::new(), OpLookup::new());
let sheet = parser.parse_str(r#"
    sheet image_resize {
        cell width:  f64 = 1920.0;
        cell height: f64 = 1080.0;
        cell area:   f64;

        relationship {
            method [width, height] -> [area]   { width * height }
            method [area, height]  -> [width]  { area / height }
            method [width, area]   -> [height] { area / width }
        }
    }
"#)?;
```

`parse_str` resets internal parse state on each call; calling it a second time on the
same `PmParser` is allowed.

## Method Body Compilation

### Argument-passing approach (no shared state)

For a method `method [width: f64, height: f64] -> [area: f64] { width * height }`:

1. The parser looks up each input name in the sheet's cell symbol table to obtain its
   concrete `TypeId`.
2. A custom `OpLookup` scope is pushed that registers each input name as a `push_arg(i,
   TypeId)` op — an operation that, at execution time, pushes the i-th call argument.
3. `CELParser` parses the body expression with this scope, producing a `DynSegment`.
4. After parsing, `DynSegment::peek_stack_infos(1)` returns the output `TypeId`, which is
   checked against the declared output cell's `TypeId`. A mismatch is a `ParseError`.
5. The `DynSegment` is owned outright by the method closure (no reference counting).

The resulting `MethodFn` closure:

```rust
let segment = RefCell::new(segment); // interior mutability; owned uniquely by the closure
Box::new(move |inputs: &[&dyn Any]| {
    let result = segment.borrow_mut().call_dyn::<f64>(inputs)?;
    Ok(vec![Box::new(result) as Box<dyn Any>])
})
```

`MethodFn` is `Fn` (not `FnMut`), so `DynSegment` is wrapped in `RefCell` to satisfy
the interior-mutability requirement of `call_dyn(&mut self)`. The `RefCell` is uniquely
owned by the closure — no `Rc`, no reference counting, no extended lifetime. Both the
`RefCell` and the `DynSegment` inside it are dropped when the `Method` is dropped.

### Multi-output tuple methods

For `method [a, b] -> [sum, diff] { (a + b, a - b) }`:

- pm-lang splits the `output_list` tuple into two `or_expression`s.
- Each is compiled independently into its own `DynSegment` using the same input scope.
- The method closure evaluates both segments with the same `inputs` slice and collects
  results into the output `Vec`.

No changes to `cel-parser` or `DynSegment` are needed for this; the tuple is handled
entirely within pm-lang's `method_body` production.

### Parse-time type checking summary

| Check | When | Error |
| ----- | ---- | ----- |
| Cell type name exists in `TypeRegistry` | cell_decl | `ParseError` |
| Literal type matches cell annotation | cell_decl | `ParseError` |
| Default available when no initializer | cell_decl | `ParseError` |
| Cell name exists when used in cell_list | method_decl | `ParseError` |
| Method output type matches body expression type | method_decl | `ParseError` |
| Conditional match cell name exists and has known type | conditional_decl | `ParseError` |
| Branch literal type matches match cell type | conditional_branch | `ParseError` |

## Changes to Existing Crates

### `cel-runtime` — DynSegment extension

Two new methods on `DynSegment`:

```rust
impl DynSegment {
    /// Emits an op that pushes the call argument at `index` onto the stack.
    ///
    /// - Precondition: `type_id` matches the type of the argument at `index`
    ///   in every future `call_dyn` call.
    ///
    /// - Complexity: O(1).
    pub fn push_arg(&mut self, index: usize, type_id: TypeId);

    /// Executes the segment with `inputs` as call arguments.
    ///
    /// `push_arg` ops read from `inputs` by index at execution time.
    ///
    /// # Errors
    ///
    /// Returns `Err` if any op returns an error, or if `R`'s `TypeId` does
    /// not match the top-of-stack type after execution.
    ///
    /// - Complexity: O(n) in the number of ops.
    pub fn call_dyn<R: 'static>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<R>;
}
```

### `cel-parser` — two edits

1. `mod lex_lexer` → `pub mod lex_lexer` in `cel-parser/src/lib.rs`, exposing `LexLexer`,
   `Token`, `TokenStreamIter`, `HasSpan`, and `PunctOp` for use by `pm-lang`.

2. Add `('-', '>')` and `('=', '>')` to `LexLexer::is_compound_operator` so that `->` and
   `=>` are emitted as single two-character `Token::Punct` tokens.

No other changes to `cel-parser`. `CELParser`, `OpLookup`, and error types are unchanged.

## Crate Structure

```text
pm-lang/
├── Cargo.toml           (deps: cel-parser, cel-runtime, property-model, proc-macro2, anyhow)
└── src/
    ├── lib.rs           pub re-exports; crate-level doc with usage tutorial
    ├── parser.rs        PmParser; all grammar productions
    ├── type_registry.rs TypeRegistry and built-in type registration
    └── error.rs         re-exports ParseError from cel-parser (no new error type)
```

`pm-lang` is added to `[workspace] members`. No existing crate depends on it.
