const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
import { describe, it } from "vitest";
globalThis.d3 = require("../assets/d3.v7.min.js");
const {
  cellEdgePoint,
  circleEdgePoint,
  linkEndpoints,
  computeBBox,
  fitTransformFor,
  reconcileNodes,
  ProposalQueue,
  GraphInstance,
  createChannel,
  nextCounter,
} = require("../assets/graph.js");

/**
 * Creates a delayed channel with observable submissions and approved frames.
 * @returns {object} Queue, messages, frames, errors and an explicit reply driver.
 */
function delayedChannel(generation = 1) {
  const messages = [], frames = [], errors = [];
  const queue = new ProposalQueue(generation, m => messages.push(m),
    output => frames.push(output.view.x), error => errors.push(error));
  const snapshot = x => ({
    revision: 1, bounds: [0, 0, 200, 100], viewport: [400, 300],
    action: { kind: "propose", view: { x, y: 50, k: 2 } },
  });
  const reply = (request = messages.filter(m => m.kind === "update").at(-1).request) => {
    queue.receive({ kind: "accepted", generation: request.generation,
      revision: request.revision, sequence: request.sequence,
      output: { view: request.action.view || { x: 0, y: 50, k: 2 },
        fit: { x: 0, y: 50, k: 2 }, min_zoom: 2, max_zoom: 8 } });
  };
  return { queue, messages, frames, errors, snapshot, reply };
}

describe("approval channel", () => {
  it("proposed_view_is_not_rendered_before_approval", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    assert.deepEqual(c.frames, []); c.reply(); assert.deepEqual(c.frames, [1]);
  });
  it("bursts_keep_one_in_flight_and_one_pending_snapshot", () => {
    const c = delayedChannel();
    for (let i = 1; i <= 100; i++) {
      c.queue.submit(c.snapshot(i));
      assert.ok(c.queue.size <= 2);
      if (i === 50) { c.reply(); assert.deepEqual(c.frames, [1]); }
    }
    c.reply(); c.reply();
    assert.equal(c.frames.at(-1), 100); assert.equal(c.queue.size, 0);
    assert.equal(c.messages.filter(m => m.kind === "update").length, 3);
  });
  it("old_revision_reply_does_not_apply", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.submit({ ...c.snapshot(2), revision: 2 }); c.reply();
    assert.deepEqual(c.frames, []);
  });
  it("approval_does_not_resubmit", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1)); c.reply();
    assert.equal(c.messages.length, 2);
  });
  it("old_revision_reply_releases_inflight_and_pumps_latest", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.submit({ ...c.snapshot(2), revision: 2 }); c.reply();
    assert.equal(c.messages.length, 3); c.reply();
    assert.deepEqual(c.frames, [2]);
  });
  it("old_geometry_failure_also_releases_and_pumps_latest", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.submit({ ...c.snapshot(2), revision: 2 });
    c.queue.receive({ kind: "failed", generation: 1, sequence: 1, revision: 1, message: "old geometry" });
    assert.equal(c.messages.length, 3); assert.deepEqual(c.errors, []);
    c.reply(); assert.deepEqual(c.frames, [2]);
  });
  it("malformed_current_reply_reports_an_error_instead_of_hanging", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.receive({ generation: "1", sequence: 1, revision: 1 });
    assert.equal(c.errors.length, 1); assert.equal(c.queue.size, 0);
  });
  it("transport_failure_cannot_escape_without_reporting", () => {
    const errors = [];
    const queue = new ProposalQueue(1, message => {
      if (message.kind !== "start") throw new Error("transport lost");
    }, () => assert.fail("must not render"), message => errors.push(message));
    assert.doesNotThrow(() => queue.submit(delayedChannel().snapshot(0)));
    assert.deepEqual(errors, ["transport lost"]);
  });
  it("refresh_does_not_drop_pending_fit", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.submit({ ...c.snapshot(2), action: { kind: "fit" } });
    c.queue.submit({ ...c.snapshot(3), revision: 2, action: { kind: "refresh" } });
    c.reply(); const request = c.messages.at(-1).request;
    assert.deepEqual(request.action, { kind: "fit" }); assert.equal(request.revision, 2);
  });
  it("new_user_gesture_can_cancel_pending_fit", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.submit({ ...c.snapshot(2), action: { kind: "fit" } });
    c.queue.submit(c.snapshot(3)); c.reply();
    assert.equal(c.messages.at(-1).request.action.view.x, 3);
  });
  it("destroyed_instance_ignores_late_reply", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(1));
    c.queue.dispose(); c.reply(); assert.deepEqual(c.frames, []);
    assert.equal(c.messages.at(-1).kind, "dispose");
  });
  it("old_generation_does_not_clear_new_inflight", () => {
    const c = delayedChannel(2); c.queue.submit(c.snapshot(1));
    c.reply({ generation: 1, revision: 1, sequence: 1, action: c.snapshot(9).action });
    assert.equal(c.queue.size, 1); assert.deepEqual(c.frames, []);
    c.reply(); assert.deepEqual(c.frames, [1]);
  });
  it("real_wire_fixture_matches_the_channel", () => {
    const fixture = require("./graph-protocol.json");
    const c = delayedChannel(); c.queue.submit(c.snapshot(0));
    assert.deepEqual(c.messages, fixture.messages.slice(0, 2));
    c.queue.receive(fixture.replies[0]); assert.deepEqual(c.frames, [0]);
  });
  it("invalid_u32_and_exhaustion_fail_explicitly", () => {
    for (const n of [-1, 1.5, 4294967296]) {
      assert.throws(() => delayedChannel(n), /u32/);
    }
    const c = delayedChannel(); c.queue.submit({ ...c.snapshot(0), revision: -1 });
    assert.equal(c.errors.length, 1); assert.deepEqual(c.frames, []);
  });
  it("current_failed_reply_is_visible_without_rendering", () => {
    const c = delayedChannel(); c.queue.submit(c.snapshot(0));
    c.queue.receive({ kind: "failed", generation: 1, revision: 1, sequence: 1, message: "bad geometry" });
    assert.deepEqual(c.errors, ["bad geometry"]); assert.deepEqual(c.frames, []);
  });
  it("counter_exhaustion_is_never_wrapped", () => {
    assert.equal(nextCounter(4294967294), 4294967295);
    assert.throws(() => nextCounter(4294967295), /u32/);
  });
});

/**
 * Creates a GraphInstance with a real queue/channel and a minimal D3 adapter.
 * @returns {object} The graph and observable renderer/baseline/transport calls.
 */
function approvedGraph() {
  const messages = [], frames = [], baselines = [];
  const channel = createChannel(m => messages.push(m), error => { throw new Error(error); });
  const graph = new GraphInstance("test", channel);
  graph.width = 400; graph.height = 300;
  graph.zoomLayer = { attr: (name, view) => { if (name === "transform") frames.push(view); } };
  graph.zoom = { scaleExtent: hints => { graph.hints = hints; } };
  graph.svg = { interrupt: () => {}, remove: () => {}, call: (_, view) => {
    baselines.push(view);
    graph.proposeView({ transform: view });
  } };
  return { graph, channel, messages, frames, baselines };
}

describe("GraphInstance approval adapter", () => {
  it("waits_for_nonzero_size_and_ignores_late_observer_after_disposal", () => {
    const messages = [];
    const channel = createChannel(m => messages.push(m), error => assert.fail(error));
    const graph = new GraphInstance("size-test", channel);
    const builds = [];
    graph.buildGraph = container => builds.push(container);
    const zero = { clientWidth: 0, clientHeight: 300 };
    graph.observeSize(zero); assert.deepEqual(builds, []);
    const real = { clientWidth: 400, clientHeight: 300 };
    graph.observeSize(real); assert.deepEqual(builds, [real]);
    graph.destroy(); graph.observeSize(real);
    assert.deepEqual(builds, [real]);
  });
  it("asynchronous_build_failure_is_reported_without_rendering_fallback", () => {
    const errors = [];
    const graph = new GraphInstance("failure-test", createChannel(() => {}, error => errors.push(error)));
    graph.buildGraph = () => { throw new Error("D3 build failed"); };
    assert.doesNotThrow(() => graph.observeSize({ clientWidth: 400, clientHeight: 300 }));
    assert.deepEqual(errors, ["D3 build failed"]);
    assert.equal(graph.queue.disposed, true);
  });
  it("only_approved_frames_render_and_baseline_sync_does_not_feedback", () => {
    const c = approvedGraph();
    c.graph.proposeView({ transform: { x: 0, y: 50, k: 2 } });
    assert.equal(c.frames.length, 0);
    c.channel.deliver(require("./graph-protocol.json").replies[0]);
    assert.equal(c.frames.length, 1); assert.equal(c.baselines.length, 1);
    assert.equal(c.messages.length, 2); assert.deepEqual(c.graph.hints, [2, 8]);
  });
  it("transition_approval_renders_but_defers_baseline_sync_until_end", () => {
    const c = approvedGraph(); c.graph.beginGesture();
    c.graph.proposeView({ transform: { x: 0, y: 50, k: 2 } });
    c.channel.deliver(require("./graph-protocol.json").replies[0]);
    assert.equal(c.frames.length, 1); assert.equal(c.baselines.length, 0);
    c.graph.endGesture(); assert.equal(c.baselines.length, 1);
    assert.equal(c.messages.length, 2);
  });
  it("replacement_generations_and_subscriptions_are_independent", () => {
    const c = approvedGraph(); c.graph.destroy();
    const replacement = new GraphInstance("test", c.channel);
    assert.equal(replacement.queue.generation, 2);
    c.channel.deliver(require("./graph-protocol.json").replies[0]);
    assert.equal(c.frames.length, 0); replacement.destroy();
    assert.equal(c.channel.subscriberCount, 0);
  });
  it("geometry_refresh_during_initial_fit_preserves_newer_gesture", () => {
    const c = approvedGraph();
    c.graph.updateZoomConstraints(true);
    c.graph.proposeView({ transform: { x: 0, y: 50, k: 3 } });
    c.graph.updateZoomConstraints();
    assert.equal(c.graph.queue.pending.action.kind, "propose");
  });
});

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

describe("browser module compatibility", () => {
  it("publishes the browser API when a CommonJS shim is also present", () => {
    const window = {};
    const module = { exports: {} };
    vm.runInNewContext(
      fs.readFileSync(new URL("../assets/graph.js", import.meta.url), "utf8"),
      { window, module },
    );

    assert.equal(typeof window.beginGraph.init, "function");
    assert.equal(typeof module.exports.cellEdgePoint, "function");
  });
});
