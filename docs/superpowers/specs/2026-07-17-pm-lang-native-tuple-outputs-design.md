# pm-lang: native tuple outputs

**Date:** 2026-07-17
**Author:** Sean Parent (via Claude)
**Status:** Approved

## Overview

`cel-parser` now has native tuple support (`tuple_or_group` in `primary_expression`,
backed by `DynSegment::make_tuple`/`peek_tuple_arity`/`tuple_index`), which closes
[#30](https://github.com/stlab/cel-rs/issues/30). The original pm-lang design
(`docs/superpowers/specs/2026-06-28-pm-lang-design.md`) predates this: it defines its
own `output_list` grammar production that hand-splits a comma list into N independent
`or_expression`s, compiled as N independent `DynSegment`s, specifically because native
tuple support didn't exist yet.

This spec removes `output_list` and routes multi-output method bodies through
`cel-parser`'s native tuple grammar instead, evaluating the body expression exactly
once per `propagate()` call regardless of output count.

## Grammar change

```diff
- method_body = "{" output_list "}".
- output_list = "(" or_expression "," or_expression { "," or_expression } ")"
-             | or_expression.
+ method_body = "{" or_expression "}".
```

`or_expression` is the full CEL grammar from `cel-parser`, which already includes
`tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")"`
at the `primary_expression` level. No new grammar is introduced in pm-lang; this is a
pure deletion.

### Semantics

- **1 output**: the body expression's result type must equal the output cell's type,
  exactly as today. A tuple-valued body (e.g. `(x,)`) is a type mismatch against the
  scalar output type — no special-casing needed, since a tuple's `TypeId` is the
  `DynTuple` marker, which cannot equal any registered scalar type.
- **N > 1 outputs**: the body expression's result must be a tuple of arity exactly `N`.
  Element `i` (in tuple order) must have the same type as output `i` (in `cell_list`
  order).
- **0 outputs**: not reachable — `cell_list = "[" identifier { "," identifier } "]"`
  requires at least one identifier.

Existing source text is unaffected: `method [a, b] -> [sum, diff] { (a + b, a - b) }`
parses identically as source, but the tuple is now built and read by `cel-parser`'s
native tuple ops instead of being split by pm-lang before either half is parsed.

## Why not re-parse per output

`DynSegment` cannot be cloned (its ops hold boxed closures in `RawSegment`'s storage),
so there's no cheap way to duplicate a compiled tuple-producing segment once per
output. Two approaches were considered:

1. **Re-parse the same token text N times**, each time appending a different
   `tuple_index(i)`. Requires no new runtime primitive, but every `propagate()` call
   re-evaluates the whole shared expression N times — for what tuples are specifically
   for (computing several outputs from one shared expression), that's the wrong
   default.
2. **Evaluate once, split the result** via a new `DynSegment` primitive. Chosen: the
   expression runs exactly once per `propagate()` regardless of output count.

## cel-runtime addition

### `RawStack::read_at`

A read-only sibling of the existing `drop_at`, giving a properly-aligned pointer
directly into the stack's buffer (no scratch-copy, so no alignment hazard when the
caller reinterprets those bytes as a concrete `T`):

```rust
/// Reads a value at absolute buffer offset `offset`, given a callback that receives
/// a pointer to its bytes.
///
/// # Safety
/// `offset` must point to a live, valid, properly-aligned value; `read` must not
/// retain the pointer beyond the call.
pub unsafe fn read_at<R>(&self, offset: usize, read: impl FnOnce(*const u8) -> R) -> R;
```

### `DynSegment::call_dyn_tuple`

```rust
/// Reads and clones a value of some fixed type from `ptr`, boxing it as `Box<dyn Any>`.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this
/// function was generated for.
pub type BoxExtractor = unsafe fn(*const u8) -> Box<dyn Any>;

impl DynSegment {
    /// Executes the segment once and splits its tuple result into one boxed value
    /// per element, using `extractors[i]` to read element `i`.
    ///
    /// - Precondition: the segment requires no pre-loaded arguments (as `call_dyn`).
    /// - Precondition: `extractors[i]` matches element `i`'s runtime type.
    ///
    /// # Errors
    /// Returns `Err` if any op fails, the result isn't a tuple, or the tuple's arity
    /// doesn't equal `extractors.len()`.
    ///
    /// - Complexity: O(n) in the number of ops, plus O(extractors.len()) to split
    ///   the result.
    pub fn call_dyn_tuple(
        &mut self,
        inputs: &[&dyn Any],
        extractors: &[BoxExtractor],
    ) -> anyhow::Result<Vec<Box<dyn Any>>>;
}
```

Implementation: validate preconditions (`argument_ids.is_empty()`, `stack_ids.len() ==
1`, top is `DynTuple`, `associated.len() == extractors.len()`) — same style as
`call_dyn`'s existing `ensure!` checks. Set the `CALL_DYN_PTR`/`CALL_DYN_LEN`
thread-locals and `DynCallGuard` exactly as `call_dyn` does, run
`self.segment.call0_stack(&mut stack)` on a fresh `RawStack` sized to
`self.segment.base_alignment()` (both already available within `dyn_segment.rs`; no
`RawSegment` changes needed), then for each associated element call
`stack.read_at(tuple_base + elem.offset, extractors[i])`, and finally drop the whole
tuple in place via the existing `drop_tuple` free function and `stack.drop_at`.

`BoxExtractor` and `call_dyn_tuple` are exported from `cel_runtime`'s crate root
alongside `DynSegment`/`DynTuple`.

Single-output methods are unaffected — they keep using today's `call_dyn::<T>` path.

## pm-lang changes

### `TypeRegistry`

`TypeEntry` gains:

```rust
/// Reads and clones a `T` from a raw pointer into a type-erased box.
pub extract_box_fn: cel_runtime::BoxExtractor,
```

populated in `register`/`register_no_default` by a generic:

```rust
unsafe fn extract_box_impl<T: Clone + 'static>(ptr: *const u8) -> Box<dyn Any> {
    Box::new(unsafe { (*ptr.cast::<T>()).clone() })
}
```

### Parser

`parse_output_list` is deleted. `parse_method_body` parses a single `or_expression`
(via the existing `parse_cel_or_expression`) and returns `(DynSegment,
CompiledOutputs)`:

```rust
enum CompiledOutputs {
    Single(CallDynFn),
    Tuple(Vec<cel_runtime::BoxExtractor>),
}
```

chosen by `outputs.len()`:

- **1**: unchanged from today — check `peek_output_type_id()` against the output's
  type, look up `call_dyn_fn`.
- **>1**: check `peek_tuple_arity()` equals `outputs.len()` (error: `"output
  expression has arity {a} but method declares {n} output(s)"`); check each element's
  `TypeId` (via `peek_stack_infos(1)[0].associated`) against the corresponding output's
  type, in the same `"output {i} \`{name}\`: type mismatch: expected \`{expected}\`,
  got \`{got}\`"` style as today's per-output check; collect `extract_box_fn` per
  output into the `Vec`.

`build_method` wraps a single `RefCell<DynSegment>` (was `Vec<RefCell<DynSegment>>`)
and dispatches on `CompiledOutputs` inside the method closure — `call_dyn` for
`Single`, `call_dyn_tuple` for `Tuple` — replacing the old per-output zip loop.

## Testing

- Existing `parse_relationship_multi_output_tuple` test (source unchanged) continues
  to pass, now exercising the native-tuple path.
- New: tuple arity mismatch (method declares 3 outputs, body yields a 2-tuple).
- New: tuple element type mismatch (element `i`'s type doesn't match output `i`'s
  declared type).
- New: single-output method whose body yields a tuple is rejected as a type mismatch.
- New (cel-runtime): `call_dyn_tuple` unit tests mirroring `call_dyn`'s existing
  coverage (happy path, arity mismatch, non-tuple result, op error propagation).

## Documentation updates

- `pm-lang/src/parser.rs` module-doc grammar block: remove `output_list`, update
  `method_body`.
- `docs/superpowers/specs/2026-06-28-pm-lang-design.md`: superseded by this spec for
  the `output_list`/"Multi-output tuple methods" sections — add a pointer note rather
  than rewriting history.
- `PmParser::parse_str` doc comment's `# Errors` list: replace "arity mismatch in an
  `output_list` tuple" with "tuple arity or element-type mismatch between the output
  expression and declared outputs".
