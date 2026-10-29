import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

/** Executes the actual browser bridge with controlled picker and file-input ports. */
function bridge() {
  const state = { picker: undefined, input: null };
  const context = {
    window: {},
    document: {
      /** Supplies only the file-input event protocol consumed by the bridge. */
      createElement(tag) {
        assert.equal(tag, "input");
        const listeners = new Map();
        return {
          files: state.input ? [state.input] : [],
          addEventListener: (name, callback) => listeners.set(name, callback),
          click: () => listeners.get(state.input ? "change" : "cancel")(),
        };
      },
    },
  };
  vm.runInNewContext(readFileSync(new URL("../assets/open_file.js", import.meta.url), "utf8"), context);
  return { api: context.window.beginOpenFile, window: context.window, state, context };
}

/** Returns a real bridge-port file whose contents can change between refreshes. */
function handle(name, state) {
  return { name, getFile: async () => ({ text: async () => state.text }) };
}

/** Reads and authorizes one candidate through the adapter's commit boundary. */
async function acceptedOpen(api) {
  const result = await api.open();
  result.commit();
  return result;
}

/** Returns a promise whose completion remains controlled by this test's owner. */
function deferred() {
  let resolve;
  const promise = new Promise((complete) => { resolve = complete; });
  return { promise, resolve };
}

test("successful opens replace stale handles and refresh re-reads current contents", async () => {
  const { api, window } = bridge();
  const state = { text: "first" };
  window.showOpenFilePicker = async () => [handle("a.adm2", state)];
  const first = await acceptedOpen(api);
  assert.equal(first.text, "first");
  state.text = "edited";
  assert.equal((await api.refresh(first.id)).text, "edited");
  const second = await acceptedOpen(api);
  assert.notEqual(first.id, second.id);
  assert.equal(await api.refresh(first.id), null);
  assert.equal(Object.keys(api.handles).length, 1);
});

test("cancel and genuine read failures preserve the last re-readable handle", async () => {
  const { api, window } = bridge();
  window.showOpenFilePicker = async () => [handle("a.adm2", { text: "retained" })];
  const opened = await acceptedOpen(api);
  window.showOpenFilePicker = async () => { throw { name: "AbortError" }; };
  assert.equal(await api.open(), null);
  window.showOpenFilePicker = async () => [{
    name: "broken.adm2", getFile: async () => { throw new Error("read denied"); },
  }];
  assert.equal((await api.open()).error, "read denied");
  assert.equal((await api.refresh(opened.id)).text, "retained");
});

test("fallback success retires the previous handle but fallback cancellation does not", async () => {
  const { api, window, state } = bridge();
  window.showOpenFilePicker = async () => [handle("old.adm2", { text: "old" })];
  const old = await acceptedOpen(api);
  window.showOpenFilePicker = async () => { throw { name: "NotAllowedError" }; };
  assert.equal(await api.open(), null);
  assert.equal((await api.refresh(old.id)).text, "old");
  state.input = { name: "fallback.adm2", text: async () => "new" };
  const opened = await acceptedOpen(api);
  assert.equal(opened.id, null);
  assert.equal(opened.text, "new");
  assert.equal(await api.refresh(old.id), null);
});

test("fallback and refresh failures resolve explicit errors instead of hanging or cancelling", async () => {
  const { api, window, state } = bridge();
  window.showOpenFilePicker = async () => [{
    name: "denied.adm2", getFile: async () => { throw { name: "NotAllowedError" }; },
  }];
  state.input = { name: "broken.adm2", text: async () => { throw new Error("input failed"); } };
  assert.equal((await api.open()).error, "input failed");
  const live = handle("live.adm2", { text: "ok" });
  window.showOpenFilePicker = async () => [live];
  const opened = await acceptedOpen(api);
  live.getFile = async () => { throw new Error("refresh failed"); };
  assert.equal((await api.refresh(opened.id)).error, "refresh failed");
  assert.equal(await api.refresh(99999), null);
});

for (const fallback of [false, true]) {
  test(`obsolete ${fallback ? "fallback" : "picker"} completion preserves the accepted refresh handle`, async () => {
    const { api, window, state } = bridge();
    const slow = deferred();
    const old = { name: "a.adm2", text: () => slow.promise };
    if (fallback) {
      state.input = old;
    } else {
      window.showOpenFilePicker = async () => [{ name: old.name, getFile: async () => old }];
    }

    const pending = api.open();
    const live = { text: "B accepted" };
    window.showOpenFilePicker = async () => [handle("b.adm2", live)];
    const accepted = await acceptedOpen(api);
    slow.resolve("A obsolete");
    const rejected = await pending;
    assert.equal(rejected.text, "A obsolete");
    live.text = "B refreshed";
    assert.equal((await api.refresh(accepted.id))?.text, "B refreshed");
    // Even an older acknowledgement delivered after the newer commit cannot retire it.
    rejected.commit();
    assert.equal((await api.refresh(accepted.id))?.text, "B refreshed");
    assert.equal(Object.keys(api.handles).length, 1);
  });
}

test("the production Open script commits only after an affirmative Rust authorization", async () => {
  const rust = readFileSync(new URL("../src/open_file.rs", import.meta.url), "utf8");
  const script = rust.match(/pub const OPEN_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  for (const authorized of [false, true]) {
    const { api, window, context } = bridge();
    window.showOpenFilePicker = async () => [handle("accepted.adm2", { text: "accepted" })];
    const accepted = await acceptedOpen(api);
    window.showOpenFilePicker = async () => [handle("candidate.adm2", { text: "candidate" })];
    const response = deferred();
    const authorization = deferred();
    context.dioxus = {
      send: (payload) => response.resolve(payload),
      recv: () => authorization.promise,
    };
    const completion = vm.runInNewContext(script, context);
    const payload = await response.promise;
    assert.equal(payload.text, "candidate");
    assert.equal((await api.refresh(accepted.id)).text, "accepted");
    assert.equal(await api.refresh(payload.id), null);
    authorization.resolve(authorized);
    await completion;
    assert.equal((await api.refresh(payload.id))?.text, authorized ? "candidate" : undefined);
    assert.equal((await api.refresh(accepted.id))?.text, authorized ? undefined : "accepted");
  }
});
