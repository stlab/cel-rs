// Mounts live inspectors and their graph containers from the generated example manifest.
// Resolve assets against the page, not this classic script's additional-js copy.
const themeBase = new URL("theme/", document.baseURI);
const moduleUrl = new URL("adam_lang_book_live.js", themeBase).href;
const manifestUrl = new URL("adam-live-examples.json", themeBase).href;
const swcUrl = new URL("swc.js", themeBase).href;
const d3Url = new URL("d3.v7.min.js", themeBase).href;
const graphJsUrl = new URL("graph.js", themeBase).href;

// Loads a page-level <script> from `url`, resolving when it has loaded. Pass `{ module: true }`
// for an ES module (e.g. swc.js); classic scripts (d3, graph.js) omit it.
function loadScript(url, { module = false } = {}) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    if (module) script.type = "module";
    script.src = url;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`adam-live: failed to load ${url}`));
    document.head.appendChild(script);
  });
}

// Creates independent script descriptors; graph-free pages need only Spectrum components.
function dependencyScripts(hasGraphs) {
  const scripts = [{ url: swcUrl, module: true }];
  if (hasGraphs) {
    scripts.push({ url: d3Url, module: false }, { url: graphJsUrl, module: false });
  }
  return scripts;
}

// Creates independent mount instructions from immutable example names and source strings.
// Throws for malformed manifest entries or graphs without an inspector.
// Time and space: O(i + g + e), for i inspectors, g graphs and e inspector-graph links.
function createMountPlan(inspectors, graphs, manifest) {
  if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error("adam-live: invalid example manifest");
  }
  if (!inspectors.every((name) => typeof name === "string" && name.length > 0)) {
    throw new Error("adam-live: inspector has no example name");
  }
  const names = new Set(inspectors);
  const graphIds = new Map();
  graphs.forEach((name, index) => {
    if (!names.has(name)) throw new Error(`adam-live: graph has no inspector for "${name}"`);
    if (!graphIds.has(name)) graphIds.set(name, []);
    graphIds.get(name).push(`adam-live-graph-${index}`);
  });
  return inspectors.map((name, index) => {
    if (!Object.hasOwn(manifest, name) || typeof manifest[name] !== "string") {
      throw new Error(`adam-live: no embedded source for "${name}"`);
    }
    return {
      id: `adam-live-${index}`,
      name,
      source: manifest[name],
      graphIds: [...(graphIds.get(name) || [])],
    };
  });
}

// Reads a manifest only after a successful HTTP response; propagates network and JSON errors.
// Time and space: O(n) for n response bytes.
async function readManifest(fetchResponse, url) {
  const response = await fetchResponse(url);
  if (!response.ok) throw new Error(`adam-live: manifest HTTP ${response.status}`);
  return response.json();
}

// Replaces caller-owned mount contents with a visible, accessible load error.
// Time and DOM space: O(n + m*n) for m mounts and n message characters.
function showLoadFailure(mounts, error) {
  const message = `Live example unavailable: ${error?.message || String(error)}`;
  mounts.forEach((div) => {
    div.setAttribute("role", "alert");
    div.textContent = message;
  });
}

// Loads page dependencies once, then mounts every validated inspector and its graphs.
// A rejected dependency or mount operation leaves a visible diagnostic in every mount.
// Time and space add DOM selection, dependency loading, and live-module mounting
// to createMountPlan's O(i + g + e) bounds.
async function mountLiveExamples() {
  const inspectorMounts = document.querySelectorAll(".adam-live");
  const graphMounts = document.querySelectorAll(".adam-live-graph");
  if (inspectorMounts.length === 0 && graphMounts.length === 0) {
    return;
  }

  try {
    const loaders = [
      import(moduleUrl),
      readManifest(fetch, manifestUrl),
      ...dependencyScripts(graphMounts.length > 0).map(({ url, module }) => loadScript(url, { module })),
    ];
    const [{ default: init, mount }, manifest] = await Promise.all(loaders);
    const plan = createMountPlan(
      Array.from(inspectorMounts, (div) => div.dataset.example),
      Array.from(graphMounts, (div) => div.dataset.example),
      manifest,
    );
    await init();
    graphMounts.forEach((div, index) => {
      div.id = `adam-live-graph-${index}`;
    });
    inspectorMounts.forEach((div, index) => {
      const item = plan[index];
      div.id = item.id;
      mount(item.id, item.source, item.name, item.graphIds);
    });
  } catch (error) {
    showLoadFailure([...inspectorMounts, ...graphMounts], error);
    console.error(error);
  }
}

mountLiveExamples();
