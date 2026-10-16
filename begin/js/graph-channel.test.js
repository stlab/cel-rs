const assert = require("node:assert/strict");
import { describe, it } from "vitest";
const { runGraphChannel } = require("../../adam-web-ui/src/graph/channel.js");
const { createChannel } = require("../assets/graph.js");

/**
 * Creates an asynchronous transport/driver harness for the actual mounted script.
 * @returns {object} Independent owner registry, controls, and observed calls.
 */
function mountedChannel() {
  const sent = [], calls = [];
  let receive;
  const transport = {
    send: value => sent.push(value),
    recv: () => new Promise(resolve => { receive = resolve; }),
  };
  const root = { d3: {}, __beginGraphData: {}, beginGraph: {
    createChannel,
    init: (id, data) => calls.push(["init", id, data]),
    destroy: id => calls.push(["destroy", id]),
  } };
  return { root, calls, sent, transport, reply: value => receive(value) };
}

describe("mounted shared graph channel", () => {
  it("preserves_the_real_dioxus_sender_receiver_binding", async () => {
    const c = mountedChannel();
    c.transport.sent = [];
    c.transport.send = function (message) { this.sent.push(message); };
    c.root.beginGraph.init = id => c.root.__beginGraphChannels[id].send({ kind: "start", generation: 1 });
    const task = runGraphChannel(c.root, "a", {}, c.transport);
    assert.deepEqual(c.transport.sent, [{ kind: "start", generation: 1 }]);
    c.reply({ kind: "close" }); await task;
  });
  it("installs_before_init_and_closes_on_unmount", async () => {
    const c = mountedChannel();
    c.root.beginGraph.init = id => {
      assert.ok(c.root.__beginGraphChannels[id]);
      c.calls.push(["init", id]);
    };
    const task = runGraphChannel(c.root, "a", { nodes: [] }, c.transport);
    assert.deepEqual(c.calls, [["init", "a"]]);
    c.reply({ kind: "close" }); await task;
    assert.deepEqual(c.calls.at(-1), ["destroy", "a"]);
    assert.equal(c.root.__beginGraphChannels.a, undefined);
  });
  it("replacement_cancels_old_owner_without_destroying_new_owner", async () => {
    const c = mountedChannel(), newer = mountedChannel();
    const oldTask = runGraphChannel(c.root, "a", {}, c.transport);
    const task = runGraphChannel(c.root, "a", { newer: true }, newer.transport);
    const channel = c.root.__beginGraphChannels.a;
    c.reply({ kind: "close" }); await oldTask;
    assert.equal(c.root.__beginGraphChannels.a, channel);
    newer.reply({ kind: "close" }); await task;
    assert.equal(c.root.__beginGraphChannels.a, undefined);
  });
  it("close_before_driver_load_aborts_polling_and_never_initializes", async () => {
    const c = mountedChannel(); const driver = c.root.beginGraph;
    delete c.root.beginGraph;
    const task = runGraphChannel(c.root, "a", {}, c.transport);
    c.reply({ kind: "close" }); await task;
    c.root.beginGraph = driver;
    await new Promise(resolve => setTimeout(resolve, 60));
    assert.deepEqual(c.calls, []);
  });
  it("waits_for_d3_as_well_as_the_driver", async () => {
    const c = mountedChannel(); delete c.root.d3;
    const task = runGraphChannel(c.root, "a", {}, c.transport);
    assert.deepEqual(c.calls, []);
    c.root.d3 = {};
    await new Promise(resolve => setTimeout(resolve, 60));
    assert.equal(c.calls[0][0], "init");
    c.reply({ kind: "close" }); await task;
  });
  it("fatal_and_local_failures_are_visible_and_sent_to_rust", async () => {
    const c = mountedChannel();
    const task = runGraphChannel(c.root, "a", {}, c.transport);
    c.root.__beginGraphChannels.a.fail("broken model");
    assert.equal(c.root.__beginGraphErrors.a, "broken model");
    assert.equal(c.sent.at(-1), "broken model");
    await task;
    const second = mountedChannel();
    const next = runGraphChannel(second.root, "a", {}, second.transport);
    second.reply({ kind: "fatal", message: "transport failed" }); await next;
    assert.equal(second.root.__beginGraphErrors.a, "transport failed");
  });
  it("broken_sender_still_closes_the_owned_channel_and_resources", async () => {
    const c = mountedChannel();
    const task = runGraphChannel(c.root, "a", {}, c.transport);
    const channel = c.root.__beginGraphChannels.a;
    c.transport.send = () => { throw new Error("transport offline"); };
    assert.throws(() => channel.fail("local failure"), /transport offline/);
    await task;
    assert.equal(c.root.__beginGraphChannels.a, undefined);
    assert.equal(channel.subscriberCount, 0);
    assert.deepEqual(c.calls.at(-1), ["destroy", "a"]);
  });
});
