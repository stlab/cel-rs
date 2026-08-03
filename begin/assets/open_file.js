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
          return null; // user cancelled the picker
        }
        const id = this.nextId++;
        this.handles[id] = handle;
        const file = await handle.getFile();
        const text = await file.text();
        return { id, name: handle.name, text };
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
        const text = await file.text();
        resolve({ id: null, name: file.name, text });
      });
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  },

  refresh(id) {
    const handle = this.handles[id];
    if (!handle) return Promise.resolve(null);
    return (async () => {
      const file = await handle.getFile();
      const text = await file.text();
      return { id, name: handle.name, text };
    })();
  },
};
