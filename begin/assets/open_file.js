// window.beginOpenFile: bridges Rust (via document::eval + dioxus.send) to
// the browser's File System Access API, with a plain <input type="file">
// fallback for browsers that don't support it (Firefox, Safari) *and* for
// browsers/embedders that expose it but then refuse to honor it (see
// `_openViaInput`'s doc comment).
//
// open()/refresh() resolve cancellation, successful file text, or a read error.
// OPEN_SCRIPT strips the candidate's commit callback for Rust's `Option<OpenResult>`:
// - `null` if the user cancelled — a silent no-op, never an error.
// - `{ id, name, text, commit }` on Open success; Refresh returns `{ id, name, text }`.
//   `id` is a number (a re-readable handle exists after commitment) or `null` (fallback:
//   one-shot only, nothing to refresh). `commit()` installs the candidate only
//   after Rust authorizes it; reading a candidate does not retire any handle.
// - `{ error }` if a read genuinely failed after the picker/refresh started
//   (permission revoked, file deleted mid-flow) — distinct from `null` so
//   Rust can still report a real failure to stderr instead of treating it
//   identically to the user just cancelling.
window.beginOpenFile = {
  handles: {},
  nextId: 0,
  committedId: -1,

  /** Reads a candidate without changing the accepted file's refresh handle.
   * Reading uses O(file bytes) time and space, excluding unbounded user/I/O latency.
   * Cancellation resolves null; read failures resolve an explicit error.
   */
  open() {
    if (this.nextId > 0xffffffff) {
      return Promise.resolve({ error: "file request ID space exhausted" });
    }
    const request = this.nextId++;
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
          // AbortError is the spec-defined name for a real user cancellation —
          // leave any existing handle alone and resolve to the silent no-op.
          // Anything else (observed: a `NotAllowedError` from some embedders —
          // see `_openViaInput`) means the picker itself couldn't run here at
          // all, so fall back rather than silently discarding a genuine error.
          if (e && e.name === "AbortError") {
            return null;
          }
          return this._openViaInput(request);
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
          return this._candidate(request, handle, handle.name, text);
        } catch (e) {
          // Some embedders expose `showOpenFilePicker` and let it resolve with
          // a real handle, but then refuse `getFile()` itself with a
          // `NotAllowedError` ("The request is not allowed by the user agent
          // or the platform in the current context") — observed in VS Code's
          // built-in Simple Browser, which hosts external pages in a nested
          // (non-outermost) browsing context that the File System Access
          // spec deliberately excludes from real filesystem access. Retry via
          // the universally-supported `<input type="file">` path instead of
          // surfacing this as a hard failure.
          if (e && e.name === "NotAllowedError") {
            return this._openViaInput(request);
          }
          return { error: String((e && e.message) || e) };
        }
      })();
    }

    return this._openViaInput(request);
  },

  /** Returns a uniquely owned candidate whose commit installs only a newer accepted request.
   * Pending candidates retain their own handles; discarding one releases that ownership.
   * Commit takes O(1) time and never retires a newer accepted handle.
   */
  _candidate(request, handle, name, text) {
    return {
      id: handle ? request : null,
      name,
      text,
      commit: () => {
        if (request <= this.committedId) return;
        this.committedId = request;
        this.handles = handle ? { [request]: handle } : {};
      },
    };
  },

  // Fallback: one-shot <input type="file">, no handle survives to refresh
  // from. Used both when `showOpenFilePicker` doesn't exist at all, and when
  // it exists but the embedding context refuses to actually honor it (see
  // callers above) — `<input type="file">` has no equivalent "outermost
  // browsing context" restriction.
  _openViaInput(request) {
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
          resolve(this._candidate(request, null, file.name, text));
        } catch (e) {
          resolve({ error: String((e && e.message) || e) });
        }
      });
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  },

  /** Re-reads the current handle or resolves null for a retired ID.
   * Reading uses O(file bytes) time and space, excluding unbounded I/O latency.
   * Failures resolve an explicit error without discarding the handle.
   */
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
