# Task 1 Report

## Files changed

- `begin/package.json`
- `begin/package-lock.json`
- `begin/assets/graph.js`
- `begin/js/graph.test.js`

`begin/assets/swc.js` was not regenerated or changed.

## RED command and output summary

Command:

```text
cd begin && node --test js/graph.test.js
```

The test failed before any production change because loading the classic browser
script in Node reached `window.beginGraph` and raised:

```text
ReferenceError: window is not defined
```

This confirmed the test could not consume the browser script through a
CommonJS seam yet.

## Implementation decisions

- Added Vitest 4 and the required `"test": "vitest run"` script.
- Kept the graph source as a classic browser IIFE while selecting
  `window` in the browser and `globalThis` in CommonJS environments.
- Added the required guarded `module.exports` object containing only the six
  tested helpers; the browser branch continues to assign the existing
  `window.beginGraph` API.
- Extracted `computeBBox`, `fitTransformFor`, and `reconcileNodes` as pure
  helpers and routed the corresponding `GraphInstance` behavior through them.
- Preserved the existing node reconciliation semantics: matching nodes retain
  object identity and position, value changes do not mark topology as changed,
  and label changes are reported through `relabeledIds`.
- Added contract tests for all required geometry, endpoint, viewport, and
  reconciliation cases. Tests load the production seam with `require()` and
  exercise only documented helper behavior.
- Added JSDoc contracts covering inputs, outputs, near-zero behavior,
  complexity, and reconciliation invariants.

## Validation commands and output summaries

1. `npm install --package-lock-only --ignore-scripts` from `begin` — completed
   successfully; lockfile updated and reported zero vulnerabilities.
2. `npm install --ignore-scripts` from `begin` — completed successfully;
   Vitest dependencies installed and reported zero vulnerabilities.
3. `node --check begin/assets/graph.js` — passed.
4. `git diff --check` — passed with no whitespace errors.
5. `npm test -- --run js/graph.test.js` from `begin` — passed:
   `1` test file, `10` tests passed.
6. `npm test` from `begin` — passed during the final targeted verification
   configuration with the same `10` tests passing.

## Self-review

- The CommonJS branch executes without a browser `window` and exports exactly
  the requested helpers.
- The browser branch remains classic-script compatible and preserves all
  existing `beginGraph` methods.
- Geometry calculations remain unchanged from the existing runtime behavior;
  the extracted viewport helpers are used by `GraphInstance`.
- No unrelated files or generated assets were changed.

## Commit hash

`75a87cd42de2b61a6e3b0303ff5aabe318de18fa`

## Unresolved concerns

None.

## Review fix round

### Files changed

- `begin/assets/graph.js`
- `begin/js/graph.test.js`
- `.superpowers/sdd/2026-09-20-begin-graph-unit-tests/task-1-report.md`

### Precise contract and test changes

- Expanded the helper contracts to document valid node shapes, geometry constants,
  near-zero behavior, exact endpoint postconditions, `FIT_MARGIN`, node-kind
  geometry, viewport fallback, fit centering/scale guarantees, and node
  reconciliation identity/position/value/label invariants.
- Documented the nested `linkKey` helper's accepted endpoint forms,
  order-independent result, and constant complexity.
- Added contract JSDoc to `GraphInstance.prototype.computeBBox`,
  `GraphInstance.prototype.fitTransformFor`, and `GraphInstance.prototype.update`.
- Added contract JSDoc to the test-only `assertPoint` helper.
- Assertions were retained because each expected coordinate is now justified by
  the documented constants and postconditions; no production behavior changed.

### Focused commands and results

1. `npm test -- --run js/graph.test.js` from `begin` — passed:
   `1` test file, `10` tests passed.
2. `node --check begin/assets/graph.js` — passed.
3. `git diff --check` — passed with no whitespace errors.

### Self-review

- All review findings are covered directly by adjacent contracts.
- The browser/CommonJS seam and `window.beginGraph` preservation are unchanged.
- No generated asset was regenerated; `begin/assets/swc.js` remains untouched.
- The focused suite still exercises only the CommonJS exports and documented
  behavior.

### Review-fix commit hash

`f8ce7bfa271193b3aa066525d8353ee264a33ade`
