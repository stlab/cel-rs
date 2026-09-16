# Lexical conventions

This book is a tutorial over the parser's actual token handling, not a CEL
specification. The lexer is Rust-token based, so whitespace mainly separates
tokens and does not otherwise change meaning.

## Spacing

Adjacent tokens must still form a valid token sequence, but spaces and newlines
are otherwise free-form. For example, the parser accepts the same expression
whether it is written on one line or spread across several.

## Comments

Ordinary `//` and `/* ... */` comments are not expression nodes. They are
recovered only by the trivia helpers used by the formatter pipeline, not by
runtime evaluation.

That means the language book should describe comments as source trivia, not as
part of the expression grammar.

## Diagnostics

Parse errors carry source spans. Those spans are stored as line/column pairs
and can be rendered in rustc style.

```rust
use annotate_snippets::Renderer;
use cel_parser::{CELParser, OpLookup};

let source = "1i32 + true";
let mut parser = CELParser::new(OpLookup::new());
let err = parser.parse_str(source).unwrap_err();

let rendered = err.format_rustc_style(source, "example.cel", 1, &Renderer::plain());
assert!(rendered.contains("example.cel"));
```

The parser's `SourceSpan` and `CELError` documentation explain the exact span
representation in
[cel-parser/src/error.rs](https://github.com/stlab/cel-rs/blob/main/cel-parser/src/error.rs),
and the trivia rules live in
[cel-parser/src/trivia.rs](https://github.com/stlab/cel-rs/blob/main/cel-parser/src/trivia.rs).
The parser tests exercise the rustc-style output path.
