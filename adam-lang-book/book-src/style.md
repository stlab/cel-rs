# Program Style

## Canonical formatting

`adam fmt`, backed by [`format_sheet`](../adam_lang/fn.format_sheet.html), applies
one canonical layout: four-space indentation, opening braces on the keyword's line,
and no space before a declaration's closing `;`:

```adam
{{#include examples/style/canonical_formatting.adm2}}
```

Formatting is *idempotent*: formatting canonical source leaves it unchanged.
It preserves comments, doc comments, and blank lines in their original positions,
including before `sheet` and before closing braces.
The language server refuses formatting when syntax errors are recorded.
Callers of `format_sheet` must supply a sheet with an empty `sheet.errors` list.

Commas separate list items; they do not terminate lists. Only singleton tuple values, types and
destructuring bindings require a terminal comma to distinguish them from grouping.
Conditional branches have no comma separator or terminator.

<!-- [knowledge]
Preserve strict LL(1) parsing when editing syntax or examples: select alternatives
from one current token and require an item after each separator. Only the required
singleton-tuple comma is terminal; optional trailing commas would change the grammar.
-->
