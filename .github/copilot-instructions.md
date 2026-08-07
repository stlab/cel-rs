# cel-rs Copilot Instructions

## Workspace and commands

This is an unreleased Rust 2024 workspace. Its public API is deliberately unstable: prefer
clean redesigns over compatibility layers or incremental patches.

```bash
# Format (required before committing; the pre-commit hook enforces it)
cargo fmt --all

# Build and test all workspace crates
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace

# Run a focused test
cargo test -p adam-rs --lib planner::tests::execution_order_respects_producer_consumer_dependency
# Or filter across workspace tests
cargo test --workspace <test_name>

# Lint all relevant targets; keep all three begin checks
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

CI currently builds and tests the workspace excluding `begin`, and checks `begin` with
`--no-default-features`. Before opening a PR, run the complete commands above and ensure
the plain build and test commands emit no warnings. Run `cargo doc --workspace --no-deps --lib`
when changing library documentation.

Activate shared hooks after cloning with `git config core.hooksPath .githooks`. Make changes in
a separate git worktree and never commit directly to `main`.

## Architecture

- The root `cel-rs` crate is a façade that re-exports `cel-runtime`, `cel-parser`, and
  `cel-rs-macros`.
- `cel-runtime` is the stack-based evaluator. It layers `RawStack` (unsafe aligned storage),
  `RawSegment` (operations and droppers), `DynSegment` (runtime type checks), and
  `Segment<Args, Stack>` (compile-time checks via heterogeneous cons-cell type lists).
  Operations use `opN`; fallible operations use `opNr`.
- `cel-parser` turns proc-macro token streams through `LexLexer` and the recursive-descent
  `CELParser` into `DynSegment`. `OpLookup` resolves custom scopes in LIFO order before static
  built-ins, overloading by arity and `TypeId`. `cel-rs-macros` currently validates CEL at
  compile time; it does not generate a native compiled CEL backend.
- The property-model stack flows `cel-parser`/`cel-runtime` -> `adam-rs` -> `adam-lang` ->
  `begin`. `adam-rs` plans and executes multi-way constraint relationships over type-erased
  cells. `adam-lang` parses and type-checks sheets into live `adam-rs` graphs. `adam-lsp`
  publishes adam-lang syntax and type diagnostics to the VS Code extension.
- `begin` is the desktop-first Dioxus property-model environment, with graph visualization and
  an Inspector. `editors/vscode-adam-lang` supplies `.adm2` syntax highlighting, formatting,
  and diagnostics via `adam-lsp`. `xtask` owns repository maintenance commands such as
  `cargo xtask fetch-assets` and `cargo xtask build-js`.

Read `docs/VISION.md` for roadmap, rationale, or cross-crate design direction. Dated feature
designs live under `docs/superpowers/specs/`.

## Repository conventions

- Write contract-style `///` documentation for every function. Use a present-tense summary
  ending in a period; add non-obvious `- Precondition:` and `- Postcondition:` bullets,
  `# Errors`, or `# Safety` as applicable. Document non-O(1) complexity. Public APIs need
  `# Examples`; modules use `//!` tutorials. Parser functions document their grammar production.
  Use `debug_assert!` for preconditions instead of documenting consequences of violations.
- Derive unit tests from public contracts, not implementation details. Cover documented errors,
  postconditions, and interface-implied edge cases; do not test unspecified precondition
  violations.
- Avoid avoidable heap allocation: pass `&str` and slices, prefer iterators or borrowed views,
  and use generics or function pointers instead of trait objects when type erasure is not needed.
  Signed integer arithmetic must use `checked_*`. Use `.op1r`/`.op2r` (and other `opNr`)
  variants for operations that can fail.
- Changes to `begin` UI require rendered verification, not only Cargo checks. Use the
  `verifying-begin-ui` skill to serve the web build and inspect a screenshot/DOM; the normal
  desktop WebView2 window is not headless-browser attachable.
- `begin/assets/swc.js` is a committed bundle. Regenerate it only when updating Spectrum or its
  bundle inputs with `cargo xtask build-js` (or `npm ci && npm run build` in `begin`), and commit
  the resulting bundle with the package manifest/lockfile changes. Node is otherwise unnecessary
  for building `begin`.
- For the VS Code extension, run its Node commands from `editors/vscode-adam-lang`; use
  `npm run compile` and `npm test` for extension changes. Build `adam-lsp` with
  `cargo build -p adam-lsp` before exercising its diagnostics.
