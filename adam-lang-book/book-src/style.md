# Chapter 11: Program Style

## 11.1 Canonical formatting

Adam ships its own formatter (`adam fmt`, backed by
[`format_sheet`](../adam_lang/fn.format_sheet.html)) with one canonical layout: 4-space
indentation, opening braces on the same line as the keyword that introduces them, and no space
before a declaration's closing `;`. Given input that doesn't already follow this layout,
formatting normalizes it:

```
{{#include examples/style/canonical_formatting.adm2}}
```

A formatter run is expected to be _idempotent_ (formatting already-canonical source
reproduces it unchanged) and preserves every comment, doc comment, and blank line exactly
where it appeared, including a file-header comment before `sheet` itself and a trailing comment
before a block's own closing `}`. `adam-lsp`'s `textDocument/formatting` handler refuses to
format a sheet with any recorded syntax error rather than guess at intent:
[`format_sheet`](../adam_lang/fn.format_sheet.html)'s own precondition is that `sheet.errors`
is empty.

A `conditional` branch's trailing `,` (the grammar allows one after each branch's closing `}`)
is always omitted by the formatter, even though the parser still accepts it on input, so
canonical Adam source never has one.
