# Conditionals

## Grammar

```text
conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
conditional_branch = (literal_pattern | "_") "=>" "{" { relationship_decl | relationship_use } "}".
literal_pattern    = [ "-" ] literal.
```

A `conditional` evaluates its *match subject*, the expression after the keyword,
and activates the branch, if any, whose literal equals the result.
Only that branch's relationships participate in resolution.
Branch bodies accept relationship declarations and component uses, but no cell
declarations or nested conditionals.
Branches have no comma separator or terminator.

## The match subject

The match subject may be a cell name (`conditional mode { ... }`) or an expression
with [deduced dependencies](expressions.md#deduced-dependencies):

```adam
{{#include examples/conditionals/multi_cell_match_subject.adm2}}
```

Every branch literal's type must match the match subject's own inferred type exactly; a sheet
mixing an `i32` match subject with a `bool` branch literal fails to parse, not to propagate.
The match subject can also be tuple-valued, but tuple expressions are not literal
patterns, so such a conditional can use only the `_` default branch.

## Forced cells

A relationship with exactly one method claims its output whenever active,
regardless of strength. The output is *forced*, as reported by `Sheet::is_forced`.
This query returns `false` for a relationship with multiple methods, even when strength
selects the same direction every round.

Host UIs commonly disable editing for forced cells because resolution overrides their writes.
The language does not require this convention; a host may accept a write and preserve it
as [shadow state](#shadow-state-forced-and-self-referencing-cells).

## Shadow state: forced and self-referencing cells

A forced cell preserves its source underneath its derived value, as
[filters](filters.md#the-raw-value-is-never-lost) and
[self-referencing methods](relationships-continued.md#self-referencing-methods) do.

```adam
{{#include examples/conditionals/forced_and_self_ref_shadow.adm2}}
```

With `mode == 0` (the declared default), the self-referencing branch is active.
Strength selects one of its two methods, `low := min(low, high)` or
`high := max(low, high)`. `low`, declared
first, is stalest, so the solver picks `low := min(low, high)`; `high` is left alone as an
ordinary source, its own `max` method never invoked. `low` and `high` start at `4` and `9`,
already satisfying `low <= high`, so nothing visibly changes.

Writing `high` to `42` and switching to `mode == 1` activates the single-method branch,
forcing `low` from `high`: `low` reads `42`, but its source is still `4`.
Writing `high` did not change `low`'s source. Switching back
to `mode == 0` reselects `low := min(low, high)` (still the stalest cell) and recomputes it
from both cells' sources, `4` and `42`, rather than the forced `42`: `low`
reads back down to `4`; `high`, never written since the first round, is still the one left as
a source, at `42`.

Writing `low` to `100` promotes it to freshest, flipping the solver's choice: now
`high := max(low, high)` is the one selected, deriving `high` and leaving `low` as the
source instead. `high` reads `100`, pulled up to match.

## The default branch and reverting to source

`_ => { ... }` matches any value not listed by a named branch and must appear last.
A named branch after `_` is a syntax error. The default branch is optional:
if the match value doesn't equal any branch's
literal, none of the conditional's relationships are active that round, and their outputs
become freely writable. Forced and self-referencing cells preserve a separate source
value, so releasing those cells restores their last external write or declared default.
Ordinary cells in reversible relationships need not retain a separate shadow; releasing
the relationship does not promise to undo their last derived value.
The [previous section](#shadow-state-forced-and-self-referencing-cells) demonstrates
the shadow-state mechanism for forced and self-referencing cells.

```adam
{{#include examples/conditionals/default_branch_and_spring_back.adm2}}
```

## Nested and chained conditionals

Sibling conditionals may read one another's outputs as match subjects or relationship inputs.
Branch bodies cannot contain conditionals. A resize dialog might use two sibling
conditionals, one gated on `resample`, the other on `resample && constrain`.
