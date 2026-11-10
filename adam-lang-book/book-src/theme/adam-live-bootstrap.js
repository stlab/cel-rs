// Mounts live inspectors and their graph containers from the generated example manifest.
(() => {
const script = document.currentScript;
const scope = script?.dataset.liveScope
  ? document.querySelector(script.dataset.liveScope)
  : document;
const sequence = globalThis.adamLiveSequence || 0;
globalThis.adamLiveSequence = sequence + 1;
const prefix = sequence ? `${sequence}-` : "";
// mdBook's public path_to_root identifies its publication root for nested chapters.
// Explicit embed bases remain page-relative; script copies may live under book-src/theme.
const publicationBase = typeof path_to_root === "string"
  ? new URL(path_to_root, document.baseURI)
  : document.baseURI;
const themeBase = script?.dataset.assetBase
  ? new URL(script.dataset.assetBase, document.baseURI)
  : new URL("theme/", publicationBase);
const moduleUrl = new URL("adam_lang_book_live.js", themeBase).href;
const manifestUrl = new URL("adam-live-examples.json", themeBase).href;
const d3Url = new URL("d3.v7.min.js", themeBase).href;
const graphJsUrl = new URL("graph.js", themeBase).href;
const runtimes = globalThis.adamLiveRuntimes ||= new Map();
const scripts = globalThis.adamLiveScripts ||= new Map();

// Initializes each module once per document, including concurrent scoped bootstraps.
function liveRuntime() {
  if (!runtimes.has(moduleUrl)) {
    runtimes.set(moduleUrl, import(moduleUrl).then(async runtime => {
      await runtime.default();
      return runtime;
    }));
  }
  return runtimes.get(moduleUrl);
}

// Loads a page-level <script> from `url`, resolving when it has loaded. Pass `{ module: true }`
// for an ES module; classic scripts (d3, graph.js) omit it.
function loadScript(url, { module = false } = {}) {
  if (scripts.has(url)) return scripts.get(url);
  const loaded = new Promise((resolve, reject) => {
    const script = document.createElement("script");
    if (module) script.type = "module";
    script.src = url;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`adam-live: failed to load ${url}`));
    document.head.appendChild(script);
  });
  scripts.set(url, loaded);
  return loaded;
}

// Creates independent graph descriptors; graph-free roots fetch no graph or Spectrum scripts.
function dependencyScripts(hasGraphs) {
  const scripts = [];
  if (hasGraphs) {
    scripts.push({ url: d3Url, module: false }, { url: graphJsUrl, module: false });
  }
  return scripts;
}

// Creates independent mount instructions from immutable example names and source strings.
// Throws for malformed entries, orphan graphs, or ambiguous graph associations.
// Diagnostic names may repeat; explicit root selectors associate their graphs.
// Time and space: O(i + g + e), for i inspectors, g graphs and e inspector-graph links.
function createMountPlan(inspectors, graphs, manifest, namespace = "") {
  if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error("adam-live: invalid example manifest");
  }
  const entries = inspectors.map((item) => typeof item === "string" ? {name:item} : item);
  if (!entries.every((item) => item && typeof item.name === "string" && item.name.length > 0)) {
    throw new Error("adam-live: inspector has no example name");
  }
  const names = new Map();
  const roots = new Map();
  entries.forEach((item, index) => {
    if (!names.has(item.name)) names.set(item.name, []);
    names.get(item.name).push(index);
    if (item.root) {
      if (roots.has(item.root)) throw new Error(`adam-live: duplicate root "${item.root}"`);
      roots.set(item.root, index);
    }
  });
  const graphIds = new Map();
  graphs.forEach((graph, index) => {
    const item = typeof graph === "string" ? {name:graph} : graph;
    const matches = names.get(item?.name) || [];
    const owner = item?.root ? roots.get(item.root) : matches.length === 1 ? matches[0] : undefined;
    if (owner === undefined || entries[owner].name !== item.name) {
      throw new Error(`adam-live: ambiguous inspector name "${item?.name}" or graph has no inspector`);
    }
    if (!graphIds.has(owner)) graphIds.set(owner, []);
    graphIds.get(owner).push(`adam-live-graph-${namespace}${index}`);
  });
  return entries.map(({name}, index) => {
    if (!Object.hasOwn(manifest, name) || typeof manifest[name] !== "string") {
      throw new Error(`adam-live: no embedded source for "${name}"`);
    }
    return {
      id: `adam-live-${namespace}${index}`,
      name,
      source: manifest[name],
      graphIds: [...(graphIds.get(index) || [])],
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
// A dependency failure affects its scoped mounts; a mount failure affects only its root.
// Time and space add DOM selection, dependency loading, and live-module mounting
// to createMountPlan's O(i + g + e) bounds.
async function mountLiveExamples() {
  if (!scope) throw new Error("adam-live: missing script scope");
  const inspectorMounts = scope.querySelectorAll(".adam-live:not([data-live-mounted])");
  const graphMounts = scope.querySelectorAll(".adam-live-graph:not([data-live-mounted])");
  if (inspectorMounts.length === 0 && graphMounts.length === 0) {
    return;
  }
  [...inspectorMounts, ...graphMounts].forEach(div => div.setAttribute("data-live-mounted", ""));

  try {
    const graphDependencies = Promise.all(
      dependencyScripts(graphMounts.length > 0).map(({url,module})=>loadScript(url,{module}))
    ).then(()=>null,error=>error);
    const loaders = [
      liveRuntime(),
      readManifest(fetch, manifestUrl),
    ];
    const [{ mount }, manifest] = await Promise.all(loaders);
    const roots = Array.from(inspectorMounts, (div, index) => ({
      name: div.dataset.example,
      root: div.dataset.liveRoot || div.id || `root-${index}`,
    }));
    const plan = createMountPlan(
      roots,
      Array.from(graphMounts, (div) => {
        let owner;
        if (!div.dataset.liveRoot && div.compareDocumentPosition) {
          inspectorMounts.forEach((inspector, index) => {
            if (inspector.dataset.example === div.dataset.example
                && (div.compareDocumentPosition(inspector) & 2)) owner = roots[index].root;
          });
        }
        return { name: div.dataset.example, root: div.dataset.liveRoot || owner };
      }),
      manifest,
      prefix,
    );
    graphMounts.forEach((div, index) => {
      div.id = `adam-live-graph-${prefix}${index}`;
      div.setAttribute("data-live-mounted", "");
    });
    await Promise.all(Array.from(inspectorMounts, async (div, index) => {
      const item = plan[index];
      div.id = item.id;
      div.setAttribute("data-live-mounted", "");
      try {
        if (item.graphIds.length) {
          const error = await graphDependencies;
          if (error) throw error;
        }
        mount(item.id, item.source, item.name, item.graphIds);
      } catch (error) {
        showLoadFailure([div, ...Array.from(graphMounts).filter(graph => item.graphIds.includes(graph.id))], error);
        console.error(error);
      }
    }));
  } catch (error) {
    showLoadFailure([...inspectorMounts, ...graphMounts], error);
    console.error(error);
  }
}

globalThis.adamLiveBootstrap = {createMountPlan, readManifest, showLoadFailure, dependencyScripts, loadScript, ready: mountLiveExamples()};
return globalThis.adamLiveBootstrap.ready;
})();
