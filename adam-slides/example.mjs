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
 * Loads a classic or module dependency and rejects failed script loads.
 * Complexity: O(n) in loaded script bytes.
 * @param {URL} url
 * @param {boolean} module
 * @returns {Promise<void>}
 */
function loadScript(url, module = false) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    if (module) script.type = "module";
    script.src = url.href;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`Failed to load ${url.href}`));
    document.head.append(script);
  });
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
    loadScript(urls.spectrum, true),
  ];
  if (options.graph) {
    document.getElementById("graph").hidden = false;
    document.body.classList.add("with-graph");
    loaders.push(loadScript(urls.d3), loadScript(urls.graph));
  }
  const [runtime, manifest] = await Promise.all(loaders);
  if (!Object.hasOwn(manifest, options.key) || typeof manifest[options.key] !== "string") {
    throw new Error(`Unknown example source: ${options.key}`);
  }
  await runtime.default();
  runtime.mount("inspector", manifest[options.key], options.key, options.graph ? ["graph"] : []);
}

if (typeof document !== "undefined") {
  const status = document.getElementById("status");
  try {
    await loadExample(parseExampleOptions(new URL(document.URL)), exampleAssetUrls(new URL(document.URL)));
    status.remove();
  } catch (error) {
    status.setAttribute("role", "alert");
    status.textContent = `Unable to load this example: ${error.message}`;
    console.error("adam-slides startup failed", error);
  }
}
