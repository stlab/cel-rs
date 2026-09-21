# Begin Graph Unit Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add contract-derived unit tests for `begin/assets/graph.js` and run them in CI.

**Architecture:** Keep `window.beginGraph` as the classic-script browser API. Expose the
documented pure helper interface only through `module.exports` when CommonJS is available, so
Vitest can exercise behavior without a browser, DOM, or D3 runtime.

**Tech Stack:** Node.js 26, npm, Vitest 4, JavaScript, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-20-begin-graph-unit-tests-design.md`

## Global Constraints

- Do not alter `window.beginGraph` or regenerate `begin/assets/swc.js`.
- Give every added or changed class and function a JSDoc contract with preconditions,
  postconditions, errors where applicable, and non-O(1) complexity.
- Derive every test exclusively from a documented contract and the exported interface.
- Run JavaScript tests once with `npm test` in CI.

---

### Task 1: Add the Graph Test Harness and Contracts

**Files:**
- Modify: `begin/package.json`
- Modify: `begin/package-lock.json`
- Modify: `begin/assets/graph.js`
- Create: `begin/js/graph.test.js`

**Interfaces:**
- Consumes: graph node objects with `id`, `kind`, `x`, `y`, optional `w`, `label`, and
  `value`; link objects with `source` and `target` identifiers or node objects.
- Produces: CommonJS exports `cellEdgePoint`, `circleEdgePoint`, `linkEndpoints`,
  `computeBBox`, `fitTransformFor`, and `reconcileNodes` when `module.exports` exists.

- [ ] **Step 1: Write the failing tests for the public helper contracts**

```js
const {
  cellEdgePoint,
  circleEdgePoint,
  linkEndpoints,
  computeBBox,
  fitTransformFor,
  reconcileNodes,
} = require("../assets/graph.js");

test("cellEdgePoint returns the nearest rectangle boundary point", () => {
  expect(cellEdgePoint(0, 0, 10, 5, 4, 3)).toEqual({ x: 6, y: 3 });
});

test("reconcileNodes preserves an existing node object and reports a relabeled id", () => {
  const existing = { id: "a", kind: "Cell", label: "before", x: 1, y: 2 };
  const result = reconcileNodes([existing], [{ id: "a", kind: "Cell", label: "after" }]);

  expect(result.nodes[0]).toBe(existing);
  expect(result.nodes[0]).toMatchObject({ label: "after", x: 1, y: 2 });
  expect(result.relabeledIds).toEqual(new Set(["a"]));
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd begin && npm test -- --run js/graph.test.js`

Expected: FAIL because the `test` script and CommonJS helper exports do not exist.

- [ ] **Step 3: Add Vitest and the one-shot test script**

```json
{
  "scripts": {
    "build": "esbuild js/spectrum-entry.js --bundle --format=esm --outfile=assets/swc.js --loader:.css=css",
    "test": "vitest run"
  },
  "devDependencies": {
    "vitest": "^4.1.6"
  }
}
```

Run `npm install --package-lock-only` from `begin` to update the lockfile without rebuilding
the committed Spectrum bundle.

- [ ] **Step 4: Extract and contract the pure behavior**

Add JSDoc contracts to the exported helpers and implement `computeBBox(nodes, hiddenNodeIds,
width, height)`, `fitTransformFor(bbox, width, height)`, and `reconcileNodes(oldNodes,
incomingNodes)`. Make `GraphInstance` delegate to these helpers. Preserve existing `x`, `y`,
and other simulation fields on retained node objects; copy only new node objects.

At the end of the IIFE, select the environment without changing the browser API:

```js
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    cellEdgePoint,
    circleEdgePoint,
    linkEndpoints,
    computeBBox,
    fitTransformFor,
    reconcileNodes,
  };
} else {
  window.beginGraph = { init, update, destroy, zoomIn, zoomOut, resetZoom, setShowInactive };
}
```

- [ ] **Step 5: Complete contract-derived test coverage**

Add tests for horizontal and vertical rectangle boundaries, zero-distance circle and rectangle
fallbacks, link endpoints for `Cell`, `Relationship`, `Conditional`, and `Branch` nodes,
visible-node bounding boxes, the empty-graph viewport fallback, fit centering and axis-limited
scale, topology changes, value-only stability, retained identity, and relabel detection.

- [ ] **Step 6: Run the focused test suite**

Run: `cd begin && npm test -- --run js/graph.test.js`

Expected: PASS with every geometry, bounds, transform, and reconciliation contract case.

- [ ] **Step 7: Commit the completed harness**

```bash
git add begin/package.json begin/package-lock.json begin/assets/graph.js begin/js/graph.test.js
git commit -m "test(begin): cover graph helpers"
```

### Task 2: Require Graph Tests and Document Contract Rules

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: `begin/package.json`'s `test` script from Task 1.
- Produces: a CI job step that fails when graph helper contracts fail, and repository-wide
  written contract/test requirements.

- [ ] **Step 1: Write the CI command locally**

Run: `cd begin && npm ci && npm test`

Expected: PASS. This reproduces the fresh-install CI path rather than relying on existing
`node_modules`.

- [ ] **Step 2: Add the CI step**

Insert this step after checkout and before Rust checks:

```yaml
- name: Run begin JavaScript tests
  working-directory: begin
  run: |
    npm ci
    npm test
```

- [ ] **Step 3: State the language-independent contract rule**

Extend `CLAUDE.md`'s documentation and unit-test sections with the rule that every class,
type, and function has an adjacent contract: type invariants at public boundaries, valid inputs,
observable postconditions, errors, and non-constant complexity. State that tests derive only
from that contract and the public interface; inspecting the implementation to write a test
requires clarifying the contract or redesigning the interface.

- [ ] **Step 4: Run the fresh-install and project validation commands**

Run:

```bash
cd begin && npm ci && npm test
cargo fmt --all --check
cargo test -p begin --no-default-features
```

Expected: all commands pass and `git status --short` does not list `begin/assets/swc.js`.

- [ ] **Step 5: Commit the CI and documentation changes**

```bash
git add .github/workflows/ci.yml CLAUDE.md
git commit -m "ci: run begin graph tests"
```
