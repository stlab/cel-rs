/**
 * Owns the mounted graph's bidirectional evaluation channel on desktop and web.
 * Installs reception before initialization, aborts driver polling on disposal,
 * and retires an old owner before reusing its container. Diagnostic strings
 * travel separately from the typed lifecycle protocol and never approve views.
 * @param {object} root Host window and container-scoped owner registries.
 * @param {string} id Container id.
 * @param {object} data Initial graph snapshot.
 * @param {{send: function(*):void, recv: function():Promise<object>}} transport Dioxus evaluation transport.
 * @returns {Promise<void>} Resolves after all owned JS resources are disposed.
 */
async function runGraphChannel(root, id, data, transport) {
  root.__beginGraphOwners = root.__beginGraphOwners || Object.create(null);
  root.__beginGraphChannels = root.__beginGraphChannels || Object.create(null);
  root.__beginGraphErrors = root.__beginGraphErrors || Object.create(null);
  root.__beginGraphData = root.__beginGraphData || Object.create(null);
  const previous = root.__beginGraphOwners[id];
  if (previous) previous.close();
  let stopped = false, timer = null, channel = null, cancelReceive;
  const cancelled = new Promise(resolve => { cancelReceive = resolve; });
  const owner = { close };
  root.__beginGraphOwners[id] = owner;
  root.__beginGraphData[id] = data;
  delete root.__beginGraphErrors[id];

  /** Retires this owner without ever destroying a replacement's instance. */
  function close() {
    if (stopped) return;
    stopped = true;
    clearTimeout(timer);
    cancelReceive({ kind: "close" });
    try {
      if (root.__beginGraphOwners[id] === owner && channel) root.beginGraph.destroy(id);
    } finally {
      if (root.__beginGraphOwners[id] === owner) {
        delete root.__beginGraphChannels[id];
        delete root.__beginGraphOwners[id];
        delete root.__beginGraphData[id];
      }
      if (channel) channel.close();
    }
  }

  /**
   * Exposes failure without policy fallback and optionally notifies Rust.
   * @param {string} message Human-readable diagnostic.
   * @param {boolean} [notify=true] Whether Rust has not already reported it.
   */
  function fail(message, notify = true) {
    root.__beginGraphErrors[id] = message;
    try { if (notify) transport.send(message); }
    finally { close(); }
  }

  /** Receives Rust replies until explicit closure or a terminal transport error. */
  async function receive() {
    try {
      while (!stopped) {
        const reply = await Promise.race([transport.recv(), cancelled]);
        if (stopped || reply.kind === "close") { close(); return; }
        if (reply.kind === "fatal") { fail(reply.message, false); return; }
        if (!channel) throw new Error("graph reply arrived before channel initialization");
        channel.deliver(reply);
      }
    } catch (error) { fail(error.message); }
  }

  /**
   * Initializes only after the driver exists, or reports bounded startup failure.
   * @param {number} remaining Number of remaining 50ms readiness probes.
   */
  function initialize(remaining) {
    if (stopped) return;
    try {
      if (root.beginGraph && root.d3) {
        channel = root.beginGraph.createChannel(function (message) { transport.send(message); }, fail);
        root.__beginGraphChannels[id] = channel;
        root.beginGraph.init(id, root.__beginGraphData[id]);
      } else if (remaining > 0) {
        timer = setTimeout(function () { initialize(remaining - 1); }, 50);
      } else {
        fail("Graph driver did not load");
      }
    } catch (error) { fail(error.message); }
  }

  const receiving = receive();
  initialize(60);
  await receiving;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { runGraphChannel };
}
