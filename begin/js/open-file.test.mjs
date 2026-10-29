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
  return { api: context.window.beginOpenFile, window: context.window, state };
}

/** Returns a real bridge-port file whose contents can change between refreshes. */
function handle(name, state) {
  return { name, getFile: async () => ({ text: async () => state.text }) };
}

test("successful opens replace stale handles and refresh re-reads current contents", async () => {
  const { api, window } = bridge();
  const state = { text: "first" };
  window.showOpenFilePicker = async () => [handle("a.adm2", state)];
  const first = await api.open();
  assert.equal(first.text, "first");
  state.text = "edited";
  assert.equal((await api.refresh(first.id)).text, "edited");
  const second = await api.open();
  assert.notEqual(first.id, second.id);
  assert.equal(await api.refresh(first.id), null);
  assert.equal(Object.keys(api.handles).length, 1);
});

test("cancel and genuine read failures preserve the last re-readable handle", async () => {
  const { api, window } = bridge();
  window.showOpenFilePicker = async () => [handle("a.adm2", { text: "retained" })];
  const opened = await api.open();
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
  const old = await api.open();
  window.showOpenFilePicker = async () => { throw { name: "NotAllowedError" }; };
  assert.equal(await api.open(), null);
  assert.equal((await api.refresh(old.id)).text, "old");
  state.input = { name: "fallback.adm2", text: async () => "new" };
  const opened = await api.open();
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
  const opened = await api.open();
  live.getFile = async () => { throw new Error("refresh failed"); };
  assert.equal((await api.refresh(opened.id)).error, "refresh failed");
  assert.equal(await api.refresh(99999), null);
});
