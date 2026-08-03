// window.beginOpenFile: bridges Rust (via document::eval + dioxus.send) to
// the browser's File System Access API, with a plain <input type="file">
// fallback for browsers that don't support it (Firefox, Safari).
//
// open()/refresh() resolve to one of three shapes, matching Rust's
// `Option<OpenResult>`:
// - `null` if the user cancelled — a silent no-op, never an error.
// - `{ id, name, text }` on success. `id` is a number (a re-readable handle
//   exists; pass it to refresh()) or `null` (the input-fallback path:
//   one-shot only, nothing to refresh).
// - `{ error }` if a read genuinely failed after the picker/refresh started
//   (permission revoked, file deleted mid-flow) — distinct from `null` so
//   Rust can still report a real failure to stderr instead of treating it
//   identically to the user just cancelling.
window.beginOpenFile = {
  handles: {},
  nextId: 0,

  open() {
    if (window.showOpenFilePicker) {
      return (async () => {
        let handle;
        try {
          [handle] = await window.showOpenFilePicker({
            types: [
              {
                description: "adam property model",
                accept: { "application/octet-stream": [".adm2"] },
              },
            ],
          });
        } catch (e) {
          return null; // user cancelled the picker — leave any existing handle alone
        }
        // A failure past this point is a genuine read failure, not a
        // cancellation, so it gets its own try/catch resolving `{ error }`
        // rather than reusing the cancel path's `null`. document::eval's
        // scripts (OPEN_SCRIPT/refresh_script) always `await` this promise
        // before calling dioxus.send(); an unhandled rejection here would
        // skip that call entirely and leave Rust's eval.recv() awaiting a
        // message that never arrives, so every path below must resolve, not
        // reject, no matter what.
        try {
          const file = await handle.getFile();
          const text = await file.text();
          // At most one file is ever "open" in this app's UI at a time, so
          // any handle from a previously opened file is now stale — drop it
          // rather than letting the map grow unbounded across a session.
          // Cleared only here, once the read has actually succeeded — not
          // right after the picker resolves (before getFile()/text() ran):
          // clearing it earlier would drop the *previous* file's still-valid
          // handle before knowing whether the new one can even be read,
          // silently turning that still-visible Refresh button into a
          // permanent no-op if this read then failed.
          const id = this.nextId++;
          this.handles = {};
          this.handles[id] = handle;
          return { id, name: handle.name, text };
        } catch (e) {
          return { error: String((e && e.message) || e) };
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
          resolve(null); // no file selected — treat like cancel, not a failure
          return;
        }
        try {
          const text = await file.text();
          resolve({ id: null, name: file.name, text });
        } catch (e) {
          resolve({ error: String((e && e.message) || e) });
        }
      });
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  },

  refresh(id) {
    const handle = this.handles[id];
    if (!handle) return Promise.resolve(null); // stale/unknown id — no-op, not a failure
    return (async () => {
      try {
        const file = await handle.getFile();
        const text = await file.text();
        return { id, name: handle.name, text };
      } catch (e) {
        return { error: String((e && e.message) || e) };
      }
    })();
  },
};
