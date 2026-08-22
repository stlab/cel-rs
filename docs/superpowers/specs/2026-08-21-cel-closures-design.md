# First-class closures in CEL, for adam-lang `filter` clauses

## Problem

adam-lang needs a way to declare a per-cell input filter (idempotent domain constraint, e.g.
clamping) directly in DSL text:

```
cell a: i32 filter |x: i32| clamp(x, 1, 100);
```

`adam-rs` already has a complete, working `Filter` mechanism (`adam-rs/src/filter.rs`,
`Sheet::add_filter`) — but only a hand-written Rust API (`Filter::from_fn_0`/`_1`/`_2`). There is no
adam-lang syntax that produces one. Closing that gap requires CEL itself to gain some notion of a
parameterized, deferred expression — a closure — since a filter's body needs a named parameter (the
candidate value) that isn't resolved until the filter actually runs.

## Background: what already exists

- **`DynSegment`** (`cel-runtime/src/dyn_segment.rs`) is the runtime's dynamically-typed compiled
  expression: a flat op list plus closure storage, called via `call_dyn`/`call_dyn_as_tuple`/
  `call_dyn_tuple_mixed`. Its arguments are supplied per call as `inputs: &[&dyn Any]` (positional),
  via a thread-local (`CALL_DYN_PTR`) that per-argument "read arg N" ops dereference — `push_arg`
  declares one such argument slot.
- **Identifier resolution is a parse-time, not run-time, concept.** `cel-parser::OpLookup` holds a
  LIFO stack of `ScopeFn` closures (`push_scope`/`pop_scope`, `op_table.rs:1487-1505`); resolving a
  name (including a bare identifier, treated as a zero-operand lookup) walks the scope stack and
  falls back to built-ins. A scope's job is to *emit ops into the segment being built* — for a plain
  identifier, that's normally "emit a read of argument slot N." Nothing is looked up dynamically by
  name at run time anywhere in the existing system.
- **`Method`/`Filter` already re-resolve their argument cells' values fresh on every call.**
  `Method` (`adam-rs/src/relationship.rs:24-30`) and `Filter` (`adam-rs/src/filter.rs:26-34`) each
  store a plain `Vec<CellId>` plus a type-erased `Box<dyn Fn(&[&dyn Any], ...) -> Result<...>>`.
  `Sheet::add_filter`/`write`/`propagate` call `cell.effective()` on each argument cell immediately
  before invoking the function, every time. The function itself never holds or caches a value — it
  only ever sees whatever it's handed for that one call. This is the existing precedent for
  "dynamic" behavior in this codebase, and closures reuse it exactly as-is rather than inventing a
  new environment/dynamic-scoping mechanism.
- **CEL tuples** (`DynTuple`, `AssociatedType`, `make_tuple`/`tuple_index` in `dyn_segment.rs`; the
  `DynamicSequence` cell type from the 2026-08-10 spec) already support a runtime-shaped, nestable
  tuple value with `.0`/`.1` field access — used below for the `a_range: (i32, i32)` example.
- **`DynExtractor`** (`dyn_segment.rs:1715`, already consumed by adam-lang's own multi-output method
  splitting in `parser.rs:1128`) is the existing mechanism for pulling a single type-erased
  `Box<dyn Any>` out of a `DynSegment`'s result without the caller knowing the concrete return type
  as a static Rust generic (`DynExtractor::Scalar(TypeId, BoxExtractor)`).

## Goals

- A genuine first-class closure **value** in `cel-runtime`: storable on the stack, passable as an
  ordinary function argument, callable via a plain Rust API — not just inline syntax sugar
  special-cased to one grammar position.
- adam-lang syntax to attach a closure-built `Filter` to a `cell` declaration, including the case
  where the filter needs other cells' current values (e.g. a tuple-typed range cell), fully
  reusing the existing `Filter`/`Sheet::add_filter` API unmodified.
- No new environment-capture or dynamic-scoping machinery: a closure never looks anything up: it is
  purely `(declared params) -> body`, and dynamism comes entirely from whoever holds and calls it
  re-supplying fresh argument values each time — exactly how `Method`/`Filter` already behave.

## Non-goals (deferred)

- General `f(x)` application syntax for an arbitrary closure-valued expression (e.g. a variable
  bound to a closure via a future `let`). Nothing in the filter use case needs this: a closure
  literal is either consumed immediately by adam-lang's `filter` clause, or (later) passed directly
  as an operand to a builtin that specifically expects and calls a closure (e.g. a future sequence
  `map`/`sort_by`). Both need only a closure *value* and a plain Rust `.call()` — no new call
  expression grammar.
- Closures as a declarable adam-lang cell *type* (storing a closure value in a cell). Not needed by
  the filter use case; the closure literal is consumed at the point it's written.
- Any support for the compile-time-checked `Segment<Args, Stack>` / `cel-rs-macros` path. Closures
  are a `DynSegment`-level (dynamically-typed) feature only, matching where adam-lang already lives.
- Any support in `AstContext` (the formatter/language-server AST-building `ParserContext` impl).
  `ParserContext::push_closure` ships with a default "unsupported" implementation and
  `AstContext` simply doesn't override it — parsing a closure literal through that path is a
  parse error until/unless a later spec adds real support.
- Recursive or mutually-recursive closure calls (see "Reentrancy" below).
- Type inference for closure parameters. Every parameter carries an explicit `: Type` annotation;
  the body compiles once, eagerly, at parse time, exactly like every other CEL body today.
- Capturing free variables from the enclosing lexical scope automatically. A closure's only names
  are its own declared parameters (plus globals/built-ins) — see "Design: adam-lang" for how a
  filter still reaches other cells despite this.

## Design

### 1. `cel-runtime`: `DynClosure`

```rust
/// A first-class, callable CEL value: a compiled body plus its declared signature.
///
/// Holds no captured environment — only its own parameters resolve inside `body`. Calling it
/// twice with different `args` is exactly as fresh each time as calling any other `DynSegment`.
pub struct DynClosure(Rc<ClosureData>);

struct ClosureData {
    param_types: Vec<TypeId>,
    return_type: TypeId,
    body: RefCell<DynSegment>,
}
```

- `Rc` is load-bearing, not a performance nicety: `DynSegment` has **no `Clone` impl** (its ops are
  boxed `dyn Fn` closures with no clone-box mechanism — giving it one would mean threading a clone
  path through every op-emission call site in the runtime, an unrelated and much larger change).
  But `DynSegment::just<T: 'static + Clone>(&mut self, value: T)` — the exact, existing mechanism a
  closure literal must go through to be embedded as a constant in whatever segment it's written
  inside — clones `value` on **every single invocation** of that segment
  (`self.op0(move || value.clone())`, `dyn_segment.rs:765-767`). `DynClosure` therefore *must*
  implement `Clone` just to be embeddable as a literal at all. `Rc<ClosureData>` is how it gets that
  `Clone` impl without `DynSegment` needing one: `Rc::clone` bumps a refcount regardless of whether
  the pointee is `Clone`, and as a side effect avoids deep-copying the whole compiled body on every
  re-execution of whatever segment holds the literal — relevant once a closure sits inside a body
  that itself re-runs on every `write`/`propagate` (the deferred, non-`filter` use case), though the
  in-scope `filter` path never actually clones it at all (see below).
- `body` needs `RefCell`: `Filter`'s `function` field is `Box<dyn Fn(...)>` (`Fn`, not `FnMut` —
  `adam-rs/src/filter.rs:33`), so the generated `wrapper_fn` that closes over a `DynClosure` can only
  reach it through `&DynClosure`, never `&mut`. `DynSegment`'s call methods (`call_dyn`,
  `call_dyn_tuple_mixed`, etc.) need `&mut self` for their own scratch stack space per call, so
  `call`'s `&self` has to reach `&mut DynSegment` through interior mutability. `RefCell` (not
  `Mutex`) matches the rest of this single-threaded runtime, which has no `Send`/`Sync` requirement
  to preserve.
- For the in-scope `filter` use case specifically, the `wrapper_fn` built in `adam-lang`'s design
  (section 3) captures one `DynClosure` by move and calls it many times over the `Filter`'s whole
  lifetime — it is never cloned. The `Clone` bound above only matters if/when a closure literal is
  used inside an ordinary CEL sub-expression that gets re-embedded via `just` (the deferred
  first-class-value use case) — it's required by that future path, not by anything `filter` does
  today. Whether `Rc<RefCell<_>>` remains the right representation once that path is actually
  built (versus giving `DynSegment` its own `Clone` impl instead) is tracked in
  [stlab/cel-rs#136](https://github.com/stlab/cel-rs/issues/136) rather than decided here.
- Unlike `DynTuple`, **no new `StackInfo`/`AssociatedType`/`RawDropper` plumbing is needed.**
  `DynTuple` exists because a tuple flattens multiple *independent* values into one aggregated
  stack region, which the existing type-checking/dropping machinery has to know how to walk
  element-by-element. `DynClosure` is a single, ordinary, opaque owned Rust value (a pointer-sized
  `Rc`) — it plugs directly into whatever generic "push a literal constant of some concrete,
  `'static` Rust type" mechanism the stack already uses for pushing e.g. a `u32` or `String`
  literal, getting a `RawDropper` generated for it the same way (`raw_dropper_for::<DynClosure>()`).

Calling one requires **no new cel-runtime primitive at all** — `DynSegment::call_dyn<R: 'static>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<R>` (`dyn_segment.rs:957`) already does exactly this: executes the segment against positional `inputs`, and its documented `# Errors` already cover "the `TypeId` of `R` does not match the top-of-stack type." `DynClosure::call` is a thin generic pass-through:

```rust
impl DynClosure {
    /// Invokes `body` with `args`, positionally matched against `param_types`.
    ///
    /// - Precondition: `args.len() == self.0.param_types.len()` and each `args[i]`'s runtime type
    ///   matches `param_types[i]` — adam-lang typechecks both before ever constructing a
    ///   `DynClosure` for a `filter` clause, so a violation here is a caller bug, not user error
    ///   (matches `push_arg`'s own existing precondition, which this delegates to unchanged).
    /// - Precondition: `TypeId::of::<R>() == self.0.return_type`.
    /// - Complexity: whatever `body`'s own evaluation complexity is.
    pub fn call<R: 'static>(&self, args: &[&dyn Any]) -> anyhow::Result<R> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        debug_assert_eq!(TypeId::of::<R>(), self.0.return_type);
        self.0.body.borrow_mut().call_dyn::<R>(args)
    }
}
```

The caller always knows `R` statically — the generated `Filter` wrapper knows the filtered cell's
concrete type, and a future builtin receiving a `DynClosure` operand would be monomorphized for a
concrete type the same way every other builtin in `op_table.rs` already is (e.g. `sig!(TYPE_I32,
...)`). `param_types`/`return_type` on `ClosureData` exist for this kind of caller-side
introspection (and the `debug_assert!`s above) — the actual runtime type safety of the call itself
is entirely `call_dyn`'s and `push_arg`'s existing responsibility, unchanged.

**Reentrancy:** `RefCell::borrow_mut()` panics if a closure's body, while running, ends up calling
back into the same closure (directly or through another closure) before the outer call returns.
Nothing in this design's scope produces that (no recursion, no closures returned from closures), so
it's called out as a known limitation rather than solved.

### 2. `cel-parser`: `|params: Type| expr` literal

Grammar addition (new primary-expression production):

```text
closure_expression = "|" [ closure_param { "," closure_param } ] "|" expression .
closure_param       = identifier ":" type_expression .
```

`|` is already tokenized (bitwise-or) but is never valid as a prefix operator, so seeing it where a
primary expression is expected is unambiguous with no lexer changes.

`type_expression` here is a *new*, cel-parser-level (not adam-lang) type annotation grammar — a bare
identifier naming one of the fixed built-in scalar types (`i32`, `f64`, `bool`, `String`, ...; the
same closed set `op_table.rs`'s `signatures_for_cast` already names for `as` casts), or a
parenthesized, recursively-nested tuple of `type_expression`s. This is new surface area: today
nothing in raw `cel-parser` needs to resolve a type *name* to a concrete Rust type outside of a cast
(`OpLookup::lookup_cast`, which converts an already-stack-resident value — it doesn't need to know
how to *declare an argument* of that type). Closures are the first feature needing that, so
`op_table.rs` gains a small table (mirroring `signatures_for_cast`'s match-by-name shape) mapping
each built-in scalar name to its `TypeId`/size/align/`RawDropper`/a `push_arg::<T>` function
pointer; a tuple `type_expression` recurses, building an `AssociatedType` list via the existing
public `cel_runtime::layout_associated` at each nesting level (the same bottom-up-then-flat
composition `dyn_segment.rs`'s private `layout_associated_recursive` already does internally, just
assembled explicitly here since that helper isn't `pub`), bound to its argument via the existing
`DynSegment::push_arg_as_dynamic_sequence_tuple`.

Compiling a closure literal:

1. Parse the parameter list, resolving each `type_expression` per the paragraph above.
2. Start a **new, independent `DynSegment`** for the body — not the segment currently being built
   around the closure literal. Mechanically, this is a new capability on `Parser<C>` itself (not
   just `ParserContext`): swap `self.context` out for `C::new_context()`, keep parsing (tokens,
   `op_lookup`, and `last_span` are untouched), then swap the original back in once the body is
   done, taking the finished fresh context as the closure's body. `ParserContext` gains one new
   method to package that finished body into a value pushed onto the (now-restored) outer context —
   `push_closure(&mut self, param_types: Vec<TypeId>, return_type: TypeId, body: Self, span: Span)`
   — with a default implementation returning "closures are not supported in this context" (so any
   future `ParserContext` impl, e.g. an AST-building context for the formatter/language server,
   need not support closures just to keep compiling), overridden by `DynSegmentContext` to build a
   `DynClosure` from `body.into_inner()` and `push_literal` it.
3. **Isolate the scope stack** before pushing the parameter scope: `OpLookup`'s `scopes: Vec<ScopeFn>`
   is LIFO with fallthrough (any enclosing scope — e.g. adam-lang's own cell-name-binding scope from
   `parse_deduced_expr` — stays reachable to whatever's pushed on top of it). Simply pushing the
   closure's own param scope on top would let a closure body silently resolve outer names too,
   quietly reintroducing the free-variable capture this design explicitly rejects. `OpLookup` gains
   `isolate_scopes(&mut self) -> Vec<ScopeFn>` (swaps `scopes` for an empty `Vec`, returning the
   displaced ones) and `restore_scopes(&mut self, scopes: Vec<ScopeFn>)`, used as a
   save/clear/restore bracket around the whole body compile. Only `push_scope` a `ScopeFn` resolving
   each parameter name to "read positional argument N" (`push_arg`) once isolated.
4. Parse `expression` via the normal grammar entry point, targeting the new segment. With the
   enclosing scope stack isolated, any name that isn't one of the closure's own parameters can only
   resolve via a built-in (`builtin_scope`, a fixed field on `OpLookup` separate from the `scopes`
   stack, so it's unaffected by isolation) or a name meaningful at parse time (e.g. `clamp`) — an
   unresolved name is a plain parse error, exactly like referencing an undeclared identifier
   anywhere else today.
5. `pop_scope` then `restore_scopes`, wrap the finished body segment plus the parameter/return
   `TypeId`s as a `DynClosure`, and push it as an ordinary literal constant onto the *outer* segment.
   The return type is read off the finished body segment's own single-result `StackInfo` (the same
   way any other expression's result type is already inferred) — closures have no return-type
   annotation in the grammar, only parameter annotations.

Because every parameter type is explicit, the body compiles exactly once, eagerly — no deferred
compilation, no per-call-site monomorphization. A closure passed as an ordinary argument to some
future builtin (`map(list, |x: i32| x + 1)`) needs no additional grammar: the builtin simply receives
a `DynClosure` value like any other operand and calls `.call(...)` inside its own `op_fn`.

### 3. `adam-lang`: the `filter` clause

Grammar addition to `parse_cell_decl` (`adam-lang/src/parser.rs:203`):

```text
cell_filter = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression .
```

Example (the motivating case — a filter that needs another cell's current value, here a
tuple-typed range that itself gets recomputed elsewhere):

```
cell a_range: (i32, i32) = (1, 100);
cell max: i32 = 100;
relate {
    a_range := (1, max);
}
cell a: i32 filter(a_range) |x: i32, r: (i32, i32)| clamp(x, r.0, r.1);
```

Compiling `cell_filter`:

1. The `(a_range)` list is resolved via the **same sibling-cell name lookup `relate`/`out` bodies
   already use** — it is adam-lang's existing name resolution, not part of the closure grammar,
   producing an ordered `Vec<CellId>`.
2. The closure literal compiles per section 2 into a `DynClosure` — here with 2 declared parameters.
3. Typecheck (`adam-lang/src/typecheck.rs`): the closure's first parameter type must match the
   filtered cell's own declared type (`i32`); each subsequent parameter type must match the
   corresponding named cell's declared type, in order (`r: (i32, i32)` ↔ `a_range: (i32, i32)`).
4. Build a `Filter` via the existing, unmodified `Filter::new(value_type, arg_cell_ids, arg_type_ids,
   wrapper_fn)`, where `wrapper_fn` is a small generated closure that downcasts the candidate value
   and each arg cell's value, then calls `dyn_closure.call(&[value, arg0, ...])` and downcasts the
   result back to the filtered cell's concrete type.
5. `Sheet::add_filter` attaches it exactly as it does today for a hand-written Rust `Filter`.

No changes to `Filter`, `Sheet`, or `add_filter` — this is purely new adam-lang syntax that
*generates* the same `Filter` value the existing Rust API already lets you build by hand. The
zero-extra-cells case (`filter |x: i32| clamp(x, 1, 100);`) is just the empty-`args` case, matching
`Filter::from_fn_0` today.

Dynamism ("the filter always sees `a_range`'s current value, never a stale one") comes entirely from
step 4/5: `Sheet` already re-reads every filter arg cell's `effective()` value fresh on each
`write`/`propagate`, exactly as it does for `Filter::from_fn_1`/`_2` today. The closure itself never
looks anything up — it only ever sees whatever `wrapper_fn` hands it for that one call.

## Error handling

- A `cell_filter`'s closure parameter types that don't match the filtered cell's type or the named
  argument cells' declared types (in order) is a typecheck diagnostic
  (`adam-lang/src/typecheck.rs`), reported the same way other cell/initializer type mismatches are
  today — not a `DynClosure` runtime error, since typechecking happens before a `DynClosure` value
  is ever built for this position.
- An identifier inside a closure body that isn't one of its own declared parameters and doesn't
  resolve to a built-in is a plain parse error at the point of the closure literal — the same
  "unresolved identifier" error path used everywhere else in `cel-parser`.
- `DynClosure::call`'s argument-count/type mismatch is a `debug_assert!`-guarded precondition, not an
  `Err` path: adam-lang's typechecker guarantees the call site always matches before a `DynClosure`
  is constructed for a filter, so a mismatch here would be an internal bug, not a user-facing error.

## Testing strategy

Per the workspace's contract-only convention:

- `cel-runtime`: `DynClosure` construction (build a small body segment by hand, wrap it), `.call()`
  with correct args returns the expected value; round-trips through `Clone` (cheap `Rc` bump, both
  clones still callable); dropped exactly once (drop-counter style test matching existing
  `dyn_segment.rs` patterns).
- `cel-parser`: parsing `|x: i32| x + 1` produces a `DynClosure` pushed as a literal; a closure body
  referencing an undeclared name is a parse error; a closure with 0 params and one with 2+ params
  both compile and call correctly; a tuple-typed parameter (`|r: (i32, i32)| r.0 + r.1`) compiles
  and calls correctly; nested closures (a closure literal appearing inside another closure's body,
  referencing only its own innermost parameters) compile and call correctly; a closure body that
  references a name bound only by an *enclosing* scope (e.g. a name adam-lang's own
  `parse_deduced_expr` scope would otherwise resolve) is a parse error, proving `isolate_scopes`
  actually blocks it rather than silently falling through.
- `adam-lang`: `cell a: i32 filter |x: i32| clamp(x, 1, 100);` parses, typechecks, and — via
  `Sheet::add_filter`/`write` — actually conforms an out-of-range value on write, matching the
  behavior of an equivalent hand-written `Filter::from_fn_0`. The `filter(a_range) |x, r| ...`
  form: typechecks argument-cell type matching (including a deliberate mismatch case, expecting a
  diagnostic), and end-to-end through `propagate()` confirms the filter's conformed output tracks
  `a_range` after it changes (proving the "fresh value every call" behavior, not a stale one from
  filter-declaration time).

## Open questions

Every fork raised during brainstorming (closure generality, capture semantics, free-variable
handling, parameter typing) was resolved above. The exact keyword/punctuation for the adam-lang
`filter(...)` extra-argument list is a small, low-risk bikeshed left to implementation planning
rather than pinned here. Whether `DynClosure` should stay `Rc<RefCell<DynSegment>>` or move to a
`DynSegment` with its own `Clone` impl is deliberately deferred, not resolved — tracked in
[stlab/cel-rs#136](https://github.com/stlab/cel-rs/issues/136) for revisit once closures have a
consumer beyond `filter`.
