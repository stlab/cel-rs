// Mounts a live SheetInspector into every `.adam-live` div the live-examples preprocessor
// inserted. Each div's `data-example` (e.g. "cells/tuple_typed_cell") names one of the
// `adam-live-examples.json` manifest's entries; the manifest and the compiled
// adam-lang-book-live wasm/js bundle are both generated into `book-src/theme/` by the book
// build (see the CI workflow changes), and mdBook's built-in theme-directory mechanism copies
// everything under `book-src/theme/` into `book-dist/theme/` verbatim (at the site root),
// regardless of whether it's also named in `book.toml`'s `additional-js`/`additional-css`
// (which this script itself is, so it is served from a different path — alongside a copy of
// `book-src/` preserved verbatim — than its sibling wasm/js/manifest files land at).
//
// A plain relative specifier can't paper over that split: `fetch()` resolves a relative URL
// against the *document's* URL, but a dynamic `import()` in a classic (non-module) script
// resolves its specifier against the *script's own* URL instead — two different base URLs in
// the same function, confirmed by a real-browser check (see this task's report). Building one
// absolute URL from `document.baseURI` and handing that same string to both calls sidesteps
// the ambiguity entirely: `import()` accepts a fully-qualified absolute URL unconditionally,
// with no dependency on which "referencing" URL it would otherwise use.
const themeBase = new URL("theme/", document.baseURI);
const moduleUrl = new URL("adam_lang_book_live.js", themeBase).href;
const manifestUrl = new URL("adam-live-examples.json", themeBase).href;

(async () => {
  const mounts = document.querySelectorAll(".adam-live");
  if (mounts.length === 0) {
    return;
  }

  const [{ default: init, mount }, manifest] = await Promise.all([
    import(moduleUrl),
    fetch(manifestUrl).then((r) => r.json()),
  ]);
  await init();

  mounts.forEach((div, index) => {
    const name = div.dataset.example;
    const source = manifest[name];
    if (source === undefined) {
      console.error(`adam-live: no embedded source for "${name}"`);
      return;
    }
    const id = `adam-live-${index}`;
    div.id = id;
    mount(id, source);
  });
})();
