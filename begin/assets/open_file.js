// window.beginOpenFile: bridges Rust (via document::eval + dioxus.send) to
// the browser's File System Access API, with a plain <input type="file">
// fallback for browsers that don't support it (Firefox, Safari).
//
// open() resolves to `{ id, name, text }` or `null` if the user cancelled.
// `id` is a number (a re-readable handle exists; pass it to refresh()) or
// `null` (the input-fallback path: one-shot only, nothing to refresh).
window.beginOpenFile = {
  handles: {},
  nextId: 0,

  open() {
    if (window.showOpenFilePicker) {
      // The whole flow (not just the picker itself) is wrapped in one
      // try/catch: a rejection anywhere here — cancelling the picker, or a
      // read failing afterward (permission revoked, file deleted mid-flow)
      // — must still resolve to null rather than reject. document::eval's
      // scripts (OPEN_SCRIPT/refresh_script) always `await` this promise
      // before calling dioxus.send(); an unhandled rejection here would
      // skip that call entirely and leave Rust's eval.recv() awaiting a
      // message that never arrives.
      return (async () => {
        try {
          const [handle] = await window.showOpenFilePicker({
            types: [
              {
                description: "adam property model",
                accept: { "application/octet-stream": [".adm2"] },
              },
            ],
          });
          const id = this.nextId++;
          // At most one file is ever "open" in this app's UI at a time, so
          // any handle from a previously opened file is now stale — drop it
          // rather than letting the map grow unbounded across a session.
          // Cleared only here, on a successful pick, not unconditionally at
          // the top of open(): clearing it before the picker resolves would
          // silently kill a still-valid handle if the user then cancelled
          // this pick.
          this.handles = {};
          this.handles[id] = handle;
          const file = await handle.getFile();
          const text = await file.text();
          return { id, name: handle.name, text };
        } catch (e) {
          return null;
        }
      })();
    }

    // Fallback: one-shot <input type="file">, no handle survives to refresh from.
    return new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".adm2";
      input.addEventListener("change", async () => {
        const file = input.files[0];
        if (!file) {
          resolve(null);
          return;
        }
        try {
          const text = await file.text();
          resolve({ id: null, name: file.name, text });
        } catch (e) {
          resolve(null);
        }
      });
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  },

  refresh(id) {
    const handle = this.handles[id];
    if (!handle) return Promise.resolve(null);
    // See open()'s comment: any failure here must resolve to null, not reject,
    // so the eval script's dioxus.send() still runs and Rust's eval.recv()
    // doesn't hang forever.
    return (async () => {
      try {
        const file = await handle.getFile();
        const text = await file.text();
        return { id, name: handle.name, text };
      } catch (e) {
        return null;
      }
    })();
  },
};
