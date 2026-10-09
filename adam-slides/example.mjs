/**
 * Returns an unambiguous safe example selector and explicit graph setting.
 * Throws for missing, repeated, or malformed parameters.
 * Complexity: O(n) in URL query length.
 * @param {URL} url
 * @returns {{ key: string, graph: boolean }}
 */
export function parseExampleOptions(url) {
  const keys = url.searchParams.getAll("example");
  const graphs = url.searchParams.getAll("graph");
  if (keys.length !== 1 || !/^tutorial\/[a-z][a-z0-9_]*$/.test(keys[0])) {
    throw new Error("Invalid or missing example selector");
  }
  if (graphs.length !== 1 || !["0", "1"].includes(graphs[0])) {
    throw new Error("Invalid or missing graph setting");
  }
  return { key: keys[0], graph: graphs[0] === "1" };
}

/**
 * Resolves runtime assets beside the host beneath any static-site prefix.
 * @param {URL} base Host page URL.
 * @returns {{ module: URL, manifest: URL, spectrum: URL, d3: URL, graph: URL }}
 */
export function exampleAssetUrls(base) {
  const theme = new URL("theme/", base);
  return {
    module: new URL("adam_lang_book_live.js", theme),
    manifest: new URL("adam-live-examples.json", theme),
    spectrum: new URL("swc.js", theme),
    d3: new URL("d3.v7.min.js", theme),
    graph: new URL("graph.js", theme),
  };
}

/**
 * Loads dependencies and mounts one sheet with its optional shared-state graph.
 * Rejects HTTP, dependency, initialization, or unknown-source failures.
 * Precondition: the host contains inspector and graph containers.
 * Complexity: O(n) in manifest, script, WebAssembly, and sheet bytes.
 * @param {{ key: string, graph: boolean }} options
 * @param {ReturnType<typeof exampleAssetUrls>} urls
 * @returns {Promise<void>}
 */
export async function loadExample(options, urls) {
  const loaders = [
    import(urls.module.href),
    fetch(urls.manifest).then(async (response) => {
      if (!response.ok) throw new Error(`Failed to fetch examples: HTTP ${response.status}`);
      return response.json();
    }),
    import(urls.spectrum.href),
  ];
  if (options.graph) {
    document.getElementById("graph").hidden = false;
    document.body.classList.add("with-graph");
    loaders.push(import(urls.d3.href), import(urls.graph.href));
  }
  const [runtime, manifest] = await Promise.all(loaders);
  for (const name of ["sp-theme", "sp-number-field", "sp-slider", "sp-checkbox"]) {
    if (!customElements.get(name)) throw new Error(`Required Spectrum element is not registered: ${name}`);
  }
  if (options.graph && (
    typeof window.d3?.select !== "function"
    || typeof window.d3?.forceSimulation !== "function"
    || typeof window.beginGraph?.init !== "function"
    || typeof window.beginGraph?.update !== "function"
  )) {
    throw new Error("Missing D3 or graph driver capabilities");
  }
  if (!Object.hasOwn(manifest, options.key) || typeof manifest[options.key] !== "string") {
    throw new Error(`Unknown example source: ${options.key}`);
  }
  await runtime.default();
  runtime.mount("inspector", manifest[options.key], options.key, options.graph ? ["graph"] : []);
}

if (typeof document !== "undefined") {
  await loadExample(parseExampleOptions(new URL(document.URL)), exampleAssetUrls(new URL(document.URL)));
  document.getElementById("status").remove();
}
