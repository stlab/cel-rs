# Chapter 9: Conditionals

## 9.1 Grammar

```text
conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
conditional_branch = (expression | "_") "=>" "{" { relationship_decl } "}" [ "," ].
```

A `conditional` evaluates its _match subject_ (the `expression` right after the
`conditional` keyword) and activates the one branch, if any, whose literal equals the current
match value. Only that branch's `relationship` blocks participate in the round's solve; every
other branch's relationships are invisible to the planner, exactly as if they weren't declared.
A branch's own body holds nothing but `relationship` declarations: a `conditional` cannot
declare cells, and cannot nest another `conditional` directly inside a branch.

## 9.2 The match subject

A bare cell name (`conditional mode { ... }`) is the common case: the match value is just that
cell's current value. The match subject can also be a general expression over several
already-declared cells, [deduced](expressions.md#44-deduced-dependencies) exactly like a
relationship binding's body:

```
{{#include examples/conditionals/multi_cell_match_subject.adm2}}
```

Every branch literal's type must match the match subject's own inferred type exactly; a sheet
mixing an `i32` match subject with a `bool` branch literal fails to parse, not to propagate.
The match subject can also be tuple-valued, in which case each branch literal is a tuple
expression of the same shape.

## 9.3 Forced cells

A relationship with exactly one method has no alternative binding to choose: its output cell
is claimed every time the sheet resolves, regardless of strength. Such a cell is _forced_
— `Sheet::is_forced` reports this, and it's `false` for a cell whose relationship has two or
more methods, even if strength happens to pick the same direction every round.

A common convention in a host UI is to disable the editable widget for a forced cell:
writing it would have no lasting effect once the sheet next resolves, so there's nothing
useful for the user to type into. This is a UI convention, not a language rule — `adam-lang`
and `adam-rs` never disable anything themselves; a host is always free to accept the write
anyway (see §9.4 below for what happens if it does).

## 9.4 Shadow state: forced and self-referencing cells

[Chapter 5](filters.md#53-the-raw-value-is-never-lost)'s filters and
[Chapter 8](relationships-continued.md#82-self-referencing-methods)'s self-referencing
methods both keep a cell's own raw *source* value forever, underneath whatever *derived*
value a live correction currently computes. A forced cell works the same way: forcing it
shadows its own source, never overwrites it.

```
{{#include examples/conditionals/forced_and_self_ref_shadow.adm2}}
```

With `mode == 0` (the declared default), the self-referencing branch is active — two methods,
`low := min(low, high)` and `high := max(low, high)`, exactly like [Chapter 7](relationships.md#72-strength-who-gets-to-stay-a-source)'s
triangle: only one is ever selected, and which one is decided by strength. `low`, declared
first, is stalest, so the solver picks `low := min(low, high)`; `high` is left alone as an
ordinary source, its own `max` method never invoked. `low` and `high` start at `4` and `9`,
already satisfying `low <= high`, so nothing visibly changes.

Writing `high` to `42` and switching to `mode == 1` activates the single-method branch,
forcing `low` from `high`: `low` reads `42`, but its own *source* is still `4` — the write
that actually changed something (`high`) never touched `low`'s source at all. Switching back
to `mode == 0` reselects `low := min(low, high)` (still the stalest cell) and recomputes it
fresh from both cells' own sources — `4` and `42` — not from the stale forced `42`: `low`
reads back down to `4`; `high`, never written since the first round, is still the one left as
a source, at `42`.

Writing `low` to `100` promotes it to freshest, flipping the solver's choice: now
`high := max(low, high)` is the one selected, deriving `high` and leaving `low` as the
source instead — `high` reads `100`, pulled up to match. Either binding can fire; which one
does is strength's call, exactly as in [Chapter 7](relationships.md#72-strength-who-gets-to-stay-a-source),
never both at once.

## 9.5 The default branch and reverting to source

`_ => { ... }` matches whatever value none of the named branches list and (because it's
defined that way) must be the last branch textually; a named branch written after `_` is a
syntax error. Leaving off `_` entirely is legal: if the match value doesn't equal any branch's
literal, none of the conditional's relationships are active that round, and every cell that
would otherwise be one of their outputs stays free, reverting to its own last
externally-written value (or its declared default), not to whatever the branch last computed
for it. This is the same shadow-state mechanism §9.4 just showed for a forced cell,
triggered here by *no* branch matching at all rather than by switching branches: a relationship
that stops being active can never have written a cell's source, so the cell has nothing to
revert to except that untouched source.

```
{{#include examples/conditionals/default_branch_and_spring_back.adm2}}
```

## 9.6 Nested and chained conditionals

Two `conditional`s in the same sheet compose freely at the sheet level: one conditional's
output cells can be another's match subject or a relationship input, exactly like any other
cell, even though a branch body itself can only hold `relationship`s, not another
`conditional`. A resize dialog, for example, might have two sibling conditionals this way: one
gated on `resample`, the other on `resample && constrain`.
