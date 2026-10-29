# Project-wide Better Code handoff

Status snapshot for the `apply-better-code` branch on 2026-10-28.
The implementation spans runtime libraries, both languages, applications,
books, the language server, the extension and maintenance tooling.
The implementation's final whole-branch review covered application code through
`eda3a78`; the subsequent `e1965a9` commit updates only Scout's synonym cache.
The pull request records validation and review of its own final head separately.

## What's done

- Corrected raw allocation length/alignment, aligned zero-sized storage,
  argument alignment, panic-time capture cleanup and overlapping stack repacking.
  Interpreted closures reuse argument-pointer storage.
- Aligned CEL parsing, checking and compilation on formal names, noncapture and
  tuple validation. Restored the precedence grammar and required grouping for
  closures used as operator operands or postfix receivers.
- Corrected Adam recovery, tuple condition checking, type-alias reverse lookup
  and erased equality. Formatters preserve comment payloads/order and share
  source indexes rather than rebuilding them throughout a formatting operation.
- Enforced exclusive unconditional Out writers and authoritative filter
  dependencies. Source-feasibility queries use the exact solver on the complete
  active component, with the last successful plan witnessing independent
  components.
- Corrected stale UI request authorization, document-scope retirement,
  exact wide-integer editing, clamp feedback, geometry hit testing and
  persistence/export validation.
- Corrected LSP FULL-sync updates and stdio shutdown ownership, extension
  activation/disposal ordering, formatter error reporting and preview-server
  path containment. Cargo example-membership tests use private source fixtures.
- Added contract-derived tests and strengthened weak result/order/ownership
  assertions. Removed duplicates only where exact or stronger behavioral
  coverage remains; retained meaningful tests for each component.
- Recorded durable runtime, grammar, planner and UI invariants in Scout knowledge.

## Significant design and API decisions

CEL retains strict Wirth-style LL(1), precedence productions and one token of
parser lookahead:

```text
expression = closure_expression | range_expression .
```

Closures are complete expressions, not primary operands. Call arguments and
collection elements accept bare closures; operators, casts and postfix suffixes
require grouping, for example `(|| 1)()`. No optional trailing commas,
backtracking or tuple-pattern extension was introduced.

Canonical stack-layout authority is sealed. Erased owners and dependent typed
segments remain thread-confined rather than inheriting unsound Send/Sync.
The public `RawStack::release_top` operation is unsafe: callers must repair the
retained physical frame before owner-based operations or unwinding.

`is_forced` remains a pure-output classification, not an exact free-source
predicate. Contributor eligibility preserves self-reference, exclusive output
claims and acyclicity. Component restriction uses the frozen successful active
set even after writes, appended declarations or failed propagation; it does not
assume an arbitrary dirty graph is feasible.

The public Adam default-branch AST gains leading-comment and blank-line fields.
Mixed/detached trivia may normalize to safe line comments while retaining payload
and order. Syntax-only tuple parsing defers resolved bounds to its owning context.

Wide-integer UI domains use exact text controls rather than JavaScript Number
widgets, including ranged/read-only values. File opening reads a candidate,
authorizes it in Rust, then commits the JavaScript handle. Displayed provenance
and the recovery/reload target are distinct. Document replacement retires
document-owned scopes and asynchronous work.

## Recorded implementation acceptance

All required workspace build/test, strict rustdoc, formatting and five Clippy
configurations passed. The final implementation run reported 3275 workspace
test passes, zero failures/ignored; the separate 527-doctest repeat is included
in that workspace count, not additional coverage.

The extension, Python, slides, graph/file-adapter and bootstrap checks reported
146 passes. Fresh runtime WASM/assets, both language books and actual application
rendering were checked during implementation. The final planner correction
was followed by 28 slides browser tests and eight live-book scenarios, with
screenshot/DOM inspection.

The first complete Talos review accepted two findings: missing UI fixture
parents with external Cargo output, fixed in `a7a21dd`, and disconnected
source-feasibility search amplification, fixed in `42ab7e8`.
All six personas returned zero observations in the complete second review;
the challenge stage accepted/rejected zero candidates and corroborated the
specific repairs. Its empty-candidate custom-rule coverage relies on the
domain reviews and parent checks, not a separate challenge read of that artifact.
Static review and passing tests do not prove absence of defects.

## Deliberately unchanged boundaries

- Exact assignment search can remain exponential inside a connected component;
  each feasibility search still scans global relationship storage. No persistent
  eligibility cache or replacement solver was added.
- Signed literal magnitude must fit before unary negation. Minimum signed
  integers remain an explicit literal/export boundary, not a lexer exception.
- Existing multi-cell conditional export limitations remain explicit errors;
  tuple-key generalization is tracked in #173.
- Requirements diagnose validity rather than rolling back accepted writes.
- Native OS dialog interaction and production native watcher delivery were not
  manually exercised; browser checks do not establish native integration.
- The existing transitive `block v0.1.6` future-incompatibility notice remains.
  The passing build/test runs had no project compiler warnings.

## What's left

No accepted implementation-review finding remains open. The PR workflow must
validate its exact final head, run its post-creation local Talos review, publish
the result and verify the saved GitHub review. Linux CI jobs remain independent
of local macOS checks; Pages deployment is push-to-main only.
No native compiled CEL backend, dependency-wide upgrade or broader language
redesign belongs to this change.
