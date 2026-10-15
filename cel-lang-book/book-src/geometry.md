# Optional Euclid geometry

`cel-euclid` adds concrete two-dimensional geometry without adding a dependency
to the CEL parser/runtime. A host opts in by calling `cel_euclid::install` on
its `OpLookup`. Install `cel-std` separately for scalar `min`, `max`, `abs`, and
other standard-library operations. Either installation order preserves both
scalar and geometry signatures.

This checked example uses reversed record-field order and a noncommuting
transform composition:

```rust
{{#include ../tests/geometry.rs:geometry}}
```

Run `cargo test -p cel-lang-book --test geometry` to execute the included code.
Installation makes record construction and named operations available. Registered
types in closure annotations additionally need a configured `TypeResolver`;
Adam's optional geometry adapter supplies that environment.

{{#include ../../cel-euclid/SIGNATURES.md}}

## Geometry policy belongs in the consuming sheet

The library supplies general geometry, not a crop-specific constraint function.
The [Adam geometry chapter](https://stlab.github.io/cel-rs/book/geometry.html)
composes inverse bounds and scalar operations into a self-referencing fitting
relationship, including numerical requirements and a fixed-overlay presentation.
The library's `examples/crop_straighten.adm2` and `examples/pan_zoom.adm2`
are executable demonstrations, not a migration of Begin's live graph.
