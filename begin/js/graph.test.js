const assert = require("node:assert/strict");
import { describe, it } from "vitest";
const {
  cellEdgePoint,
  circleEdgePoint,
  linkEndpoints,
  computeBBox,
  fitTransformFor,
  reconcileNodes,
} = require("../assets/graph.js");

/**
 * Asserts that two finite Cartesian points are equal within one nanounit.
 * @param {{x: number, y: number}} actual Point returned by a graph helper.
 * @param {{x: number, y: number}} expected Contractually expected point.
 * @postcondition The test throws when either coordinate differs by at least
 * `1e-9`; otherwise it returns normally.
 * @complexity O(1) time and space.
 */
function assertPoint(actual, expected) {
  assert.ok(Math.abs(actual.x - expected.x) < 1e-9);
  assert.ok(Math.abs(actual.y - expected.y) < 1e-9);
}

describe("cellEdgePoint", () => {
  it("finds horizontal, vertical, and diagonal rectangle intersections", () => {
    assertPoint(cellEdgePoint(0, 0, 10, 0, 5, 3), { x: 5, y: 0 });
    assertPoint(cellEdgePoint(0, 0, 0, 10, 5, 3), { x: 0, y: 7 });
    assertPoint(cellEdgePoint(0, 0, 10, 10, 5, 3), {
      x: 7,
      y: 7,
    });
  });

  it("returns the target for a near-zero source-to-target distance", () => {
    assertPoint(cellEdgePoint(4, 5, 4.5, 5.5, 5, 3), {
      x: 4.5,
      y: 5.5,
    });
  });
});

describe("circleEdgePoint", () => {
  it("finds the radial boundary intersection", () => {
    assertPoint(circleEdgePoint(0, 0, 10, 0, 3), { x: 7, y: 0 });
  });

  it("returns the center for a near-zero source-to-center distance", () => {
    assertPoint(circleEdgePoint(4, 5, 4.5, 5.5, 3), { x: 4.5, y: 5.5 });
  });
});

describe("linkEndpoints", () => {
  it("clips cell, relationship, conditional, and branch endpoints", () => {
    const cell = { id: "cell", kind: "Cell", x: 0, y: 0 };
    const relationship = {
      id: "relationship",
      kind: "Relationship",
      x: 100,
      y: 0,
    };
    const conditional = {
      id: "conditional",
      kind: "Conditional",
      x: 200,
      y: 0,
    };
    const branch = { id: "branch", kind: "Branch", x: 300, y: 0 };

    assert.deepEqual(linkEndpoints({ source: cell, target: relationship }), {
      x1: 30,
      y1: 0,
      x2: 84,
      y2: 0,
    });
    assert.deepEqual(linkEndpoints({ source: relationship, target: conditional }), {
      x1: 116,
      y1: 0,
      x2: 171.7157287525381,
      y2: 0,
    });
    assert.deepEqual(linkEndpoints({ source: conditional, target: branch }), {
      x1: 228.2842712474619,
      y1: 0,
      x2: 300,
      y2: 0,
    });
  });
});

describe("computeBBox", () => {
  it("includes visible node geometry and the layout margin", () => {
    assert.deepEqual(
      computeBBox(
        [
          { id: "cell", kind: "Cell", x: 50, y: 60, w: 80 },
          { id: "relationship", kind: "Relationship", x: 150, y: 100 },
        ],
        400,
        300,
      ),
      { minX: -6, minY: 26, maxX: 182, maxY: 132 },
    );
  });

  it("ignores hidden nodes and falls back to the viewport when empty", () => {
    assert.deepEqual(
      computeBBox(
        [
          { id: "hidden", kind: "Cell", x: 50, y: 60, w: 80 },
          { id: "visible", kind: "Branch", x: 150, y: 100 },
        ],
        400,
        300,
        new Set(["hidden"]),
      ),
      { minX: 134, minY: 84, maxX: 166, maxY: 116 },
    );
    assert.deepEqual(computeBBox([], 400, 300), {
      minX: 0,
      minY: 0,
      maxX: 400,
      maxY: 300,
    });
  });
});

describe("fitTransformFor", () => {
  it("centers the box and limits scale by the smaller viewport axis", () => {
    assert.deepEqual(
      fitTransformFor({ minX: 0, minY: 0, maxX: 200, maxY: 100 }, 400, 300),
      { fitScale: 2, x: 0, y: 50 },
    );
  });
});

describe("reconcileNodes", () => {
  it("detects topology changes while preserving identity and position", () => {
    const existing = [
      { id: "a", kind: "Cell", x: 10, y: 20, label: "A", value: "1" },
    ];
    const result = reconcileNodes(
      { nodes: existing, links: [] },
      {
        nodes: [
          { id: "a", kind: "Cell", x: 999, y: 999, label: "A", value: "2" },
          { id: "b", kind: "Relationship", x: 30, y: 40 },
        ],
        links: [{ source: "a", target: "b" }],
      },
    );

    assert.equal(result.structureChanged, true);
    assert.equal(result.nodes[0], existing[0]);
    assert.deepEqual({ x: result.nodes[0].x, y: result.nodes[0].y }, { x: 10, y: 20 });
  });

  it("keeps value-only updates stable and reports relabeled ids", () => {
    const existing = [
      { id: "a", kind: "Cell", x: 10, y: 20, label: "A", value: "1" },
    ];
    const result = reconcileNodes(
      { nodes: existing, links: [] },
      {
        nodes: [{ id: "a", kind: "Cell", x: 999, y: 999, label: "Renamed", value: "2" }],
        links: [],
      },
    );

    assert.equal(result.structureChanged, false);
    assert.deepEqual(result.relabeledIds, new Set(["a"]));
    assert.equal(result.nodes[0], existing[0]);
    assert.deepEqual(
      { x: result.nodes[0].x, y: result.nodes[0].y, label: result.nodes[0].label, value: result.nodes[0].value },
      { x: 10, y: 20, label: "Renamed", value: "2" },
    );
  });
});
