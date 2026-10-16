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
  let receive, rejectReceive;
  const transport = {
    send: value => sent.push(value),
    recv: () => new Promise((resolve, reject) => { receive = resolve; rejectReceive = reject; }),
  };
  const root = { d3: {}, __beginGraphData: {}, beginGraph: {
    createChannel,
    init: (id, data) => calls.push(["init", id, data]),
    destroy: id => calls.push(["destroy", id]),
  } };
  return { root, calls, sent, transport, reply: value => receive(value),
    reject: error => rejectReceive(error) };
}

describe("mounted shared graph channel", () => {
  it.each([false, true])("container_failure_rendering_respects_external_ownership_%s", async external => {
    const c = mountedChannel();
    const children = [];
    const container = {
      appendChild: node => children.push(node),
      querySelectorAll: () => children.slice(),
    };
    c.root.document = {
      getElementById: id => id === "book-a" ? container : null,
      createElement: () => ({
        setAttribute(name, value) { this[name] = value; },
        remove() { children.splice(children.indexOf(this), 1); },
      }),
    };
    const old = runGraphChannel(c.root, "book-a", {}, c.transport, external);
    c.root.__beginGraphChannels["book-a"].fail("intentional book failure");
    await old;
    assert.equal(children.length, Number(external));
    if (external) {
      assert.equal(children[0].role, "alert");
      assert.equal(children[0].textContent, "Graph view failed: intentional book failure");
    }
    const next = runGraphChannel(c.root, "book-a", {}, c.transport, external);
    assert.equal(children.length, 0);
    c.root.__beginGraphOwners["book-a"].close();
    await next;
  });
  it("two_simultaneous_book_owners_keep_channels_and_failures_independent", async () => {
    const c = mountedChannel(), other = mountedChannel();
    const a = runGraphChannel(c.root, "book-a", { a: true }, c.transport);
    const b = runGraphChannel(c.root, "book-b", { b: true }, other.transport);
    const channel = c.root.__beginGraphChannels["book-b"];
    const received = [];
    channel.subscribe(value => received.push(value));
    c.root.__beginGraphChannels["book-a"].fail("only a failed");
    await a;
    assert.equal(c.root.__beginGraphChannels["book-b"], channel);
    other.reply({ kind: "accepted" });
    await Promise.resolve(); await Promise.resolve();
    assert.deepEqual(received, [{ kind: "accepted" }]);
    assert.equal(c.root.__beginGraphErrors["book-b"], undefined);
    c.root.__beginGraphOwners["book-b"].close();
    await b;
  });
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
  it.each(["closed", "replaced"])("retired_receive_failure_is_ignored_when_%s", async retirement => {
    const c = mountedChannel(), newer = mountedChannel();
    const oldTask = runGraphChannel(c.root, "a", {}, c.transport);
    const oldChannel = c.root.__beginGraphChannels.a;
    c.reject(new Error("retired transport failed"));
    let task;
    if (retirement === "replaced") {
      task = runGraphChannel(c.root, "a", { newer: true }, newer.transport);
    } else {
      c.root.__beginGraphOwners.a.close();
    }
    const channel = c.root.__beginGraphChannels.a;
    try {
      await oldTask;
      assert.equal(c.root.__beginGraphErrors.a, undefined);
      assert.deepEqual(c.sent, []);
      assert.deepEqual(newer.sent, []);
      assert.equal(c.root.__beginGraphChannels.a, channel);
      assert.equal(oldChannel.subscriberCount, 0);
      assert.deepEqual(c.calls.filter(call => call[0] === "destroy"), [["destroy", "a"]]);
    } finally {
      c.root.__beginGraphOwners.a?.close();
      await task;
    }
  });
  it("current_receive_failure_is_visible_notified_and_closes_resources", async () => {
    const c = mountedChannel();
    const task = runGraphChannel(c.root, "a", {}, c.transport);
    const channel = c.root.__beginGraphChannels.a;
    c.reject(new Error("current transport failed"));
    await task;
    assert.equal(c.root.__beginGraphErrors.a, "current transport failed");
    assert.deepEqual(c.sent, ["current transport failed"]);
    assert.equal(c.root.__beginGraphChannels.a, undefined);
    assert.equal(c.root.__beginGraphOwners.a, undefined);
    assert.equal(c.root.__beginGraphData.a, undefined);
    assert.equal(channel.subscriberCount, 0);
    assert.deepEqual(c.calls.at(-1), ["destroy", "a"]);
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
