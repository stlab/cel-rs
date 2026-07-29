# cel-rs

[![CI](https://github.com/stlab/cel-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/stlab/cel-rs/actions/workflows/ci.yml)
[![Docs](https://github.com/stlab/cel-rs/actions/workflows/docs.yml/badge.svg)](https://github.com/stlab/cel-rs/actions/workflows/docs.yml)

A stack-based runtime for developing domain specific languages, paired with a
recursive-descent parser for CEL (Common Expression Language) and a proc-macro crate
for compile-time CEL validation. See [docs/VISION.md](docs/VISION.md) for the
long-term direction behind each crate in this workspace.

> **Status:** this project has not been released and has no clients yet. It is in
> active development and the API may change at any time.

## Workspace layout

| Crate | Role |
| --- | --- |
| [`cel-rs`](src/lib.rs) | Root façade crate — re-exports the crates below |
| [`cel-runtime`](cel-runtime) | Core stack-based runtime; all evaluation and stack machinery |
| [`cel-parser`](cel-parser) | Recursive-descent CEL parser, lexer, and error types |
| [`cel-rs-macros`](cel-rs-macros) | Proc-macro crate for compile-time CEL expression validation |
| [`property-model`](property-model) | Multi-way constraint system for property models |
| [`pm-lang`](pm-lang) | DSL that expresses `property-model` constraint systems as source text |
| [`pm-lsp`](pm-lsp) | Language server for `pm-lang` |
| [`begin`](begin) | Dioxus-based UI application for developing property models |
| [`xtask`](xtask) | Repository automation and maintenance tasks |

## Getting started

### Prerequisites

- Rust (stable), installed via [rustup](https://rustup.rs/)

### Clone and build

```bash
git clone https://github.com/stlab/cel-rs.git
cd cel-rs
git config core.hooksPath .githooks   # activate the shared git hooks (one-time)
cargo build --workspace
cargo test --workspace
```

### Add cel-rs to your project

```bash
cargo add cel-rs
```

### A first expression

```rust
use cel_rs::runtime::Segment;

// Build a segment that takes a u32 and a &str as arguments.
let segment = Segment::<(u32, &str)>::new()
    .op1r(|s| {
        let r = s.parse::<u32>()?;
        Ok(r)
    })
    .op2(|a, b| a + b)
    .op1(|r| r.to_string());
assert_eq!(segment.call((1u32, "2")).unwrap(), "3");
```

This builds a small pipeline over `cel-runtime`'s typed stack: parse the `&str` into
a `u32` (fallibly, via `op1r`), add it to the `u32` argument, then format the result.
`cel-parser` and `cel-rs-macros` cover the other two ways to produce a segment: parsing
CEL source at runtime, and validating/compiling CEL expressions at compile time. See
[`src/lib.rs`](src/lib.rs) for one example of each.

Full API documentation for every crate in the workspace is published from `main` at
**<https://stlab.github.io/cel-rs/>**.

## Development

See [CLAUDE.md](CLAUDE.md) for the full command reference (build, test, lint,
sanitizers) and repository conventions, including the requirement to work in a
separate git worktree for any change.

### begin: Spectrum Web Components bundle

`begin/assets/swc.js` is a single bundle (elements + Spectrum 2 theme tokens + the
zoom-control icons) produced by esbuild from real npm packages. It's committed like
every other vendored asset, so cloning and building `begin` needs no Node/npm setup.

Node.js + npm are only needed if you're updating the Spectrum version or otherwise
need to regenerate `begin/assets/swc.js`:

```bash
cargo xtask build-js
```

or directly:

```bash
cd begin
npm ci
npm run build
```

Commit the regenerated `begin/assets/swc.js` along with any `begin/package.json`/
`begin/package-lock.json` changes. See
[docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md](docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md)
for why this needs to be one compiled bundle rather than separate vendored/live files.
