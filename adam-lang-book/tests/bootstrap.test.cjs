// Exercises the authored bootstrap's decisions without a browser or external dependencies.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const vm = require("node:vm");

// Loads the real classic script with a caller-owned page and blocked external module loading.
// Records fetches, script insertions, and VM imports; requires --experimental-vm-modules.
// Time and space follow script evaluation and the supplied page's mount counts.
function bootstrap(inspectors = [], graphs = [], {runtime=false, scriptError=false, pageUrl, currentScript, pathToRoot} = {}) {
  const loading = { fetches: [], scripts: [], imports: [], mounts: [] };
  const context = vm.createContext({
    URL,
    path_to_root: pathToRoot,
    document: {
      baseURI: pageUrl || "https://example.test/book/chapter.html",
      currentScript,
      querySelectorAll: (selector) => selector.startsWith(".adam-live:") ? inspectors : graphs,
      createElement: () => ({}),
      head: {
        // Records the requested script and completes its load synchronously.
        appendChild(script) {
          loading.scripts.push(script.src);
          if (scriptError) script.onerror(); else script.onload();
        },
      },
    },
    // Records a manifest request and returns an unsuccessful HTTP response.
    fetch: async (url) => {
      loading.fetches.push(url);
      return runtime ? {ok:true, json:async()=>({a:"sheet a {}", b:"sheet b {}"})}
        : { ok: false, status: 503 };
    },
    console: {
      // Discards expected page-level errors in the isolated test context.
      error() {},
    },
  });
  context.ready = vm.runInContext(
    readFileSync(join(__dirname, "../book-src/theme/adam-live-bootstrap.js"), "utf8"),
    context,
    {
      // Records the live-module import before preventing external code execution.
      importModuleDynamically: async (url) => {
        loading.imports.push(url);
        if (runtime) {
          const module = new vm.SyntheticModule(["default","mount"], function() {
            this.setExport("default",async()=>{});
            this.setExport("mount",id=>loading.mounts.push(id));
          }, {context});
          await module.link(()=>{});
          await module.evaluate();
          return module;
        }
        throw new Error("blocked module");
      },
    },
  );
  return {
    ...vm.runInContext("({ ...adamLiveBootstrap, document })", context),
    loading,
  };
}

// Creates an independent caller-owned DOM stand-in with visible attribute and text state.
function createMount(example = "a") {
  return {
    dataset: { example },
    // Records an attribute on this mount, never on another fixture.
    setAttribute(name, value) { this[name] = value; },
  };
}

test("mount plans assign each graph to one inspector and preserve graph order", () => {
  const { createMountPlan } = bootstrap();
  const plan = createMountPlan(["a", "b"], ["a", "b", "a"], { a: "sheet a {}", b: "sheet b {}" });
  const another = createMountPlan(["a", "b"], ["a", "b", "a"], { a: "sheet a {}", b: "sheet b {}" });
  assert.deepEqual(Array.from(plan[0].graphIds), ["adam-live-graph-0", "adam-live-graph-2"]);
  assert.deepEqual(Array.from(plan[1].graphIds), ["adam-live-graph-1"]);
  plan[0].graphIds.push("changed");
  assert.equal(plan[1].graphIds.length, 1);
  assert.deepEqual(Array.from(another[0].graphIds), ["adam-live-graph-0", "adam-live-graph-2"]);
  another[0].graphIds.splice(0, 1);
  assert.deepEqual(Array.from(plan[0].graphIds), ["adam-live-graph-0", "adam-live-graph-2", "changed"]);
  plan[1].graphIds.splice(0, 1);
  assert.equal(plan[0].graphIds.length, 3);
  assert.equal(plan[0].source, "sheet a {}");
});

test("mount plans reject graph ownership shared by duplicate inspector names", () => {
  const { createMountPlan } = bootstrap();
  assert.throws(
    () => createMountPlan(["a", "a", "b"], ["a", "b", "a"], { a: "sheet a {}", b: "sheet b {}" }),
    /ambiguous inspector name "a"/,
  );
});

test("mount plans allow duplicate diagnostic names without matching graphs", () => {
  const { createMountPlan } = bootstrap();
  const plan = createMountPlan(["a", "a"], [], { a: "sheet a {}" });
  assert.notEqual(plan[0].id, plan[1].id);
});

test("ordinary nested mdBook pages resolve assets from their published root", async () => {
  const {ready, loading} = bootstrap([createMount()], [createMount()], {
    pageUrl:"https://example.test/project/book/nested/chapter.html",
    currentScript:{src:"https://example.test/project/book/book-src/theme/adam-live-bootstrap-hash.js",dataset:{}},
    pathToRoot:"../",
  });

  await ready;
  assert.deepEqual(loading.fetches, ["https://example.test/project/book/theme/adam-live-examples.json"]);
  assert.deepEqual(loading.imports, ["https://example.test/project/book/theme/adam_lang_book_live.js"]);
  assert.deepEqual(loading.scripts, [
    "https://example.test/project/book/theme/d3.v7.min.js",
    "https://example.test/project/book/theme/graph.js",
  ]);
});

test("an explicit embed asset base stays page-relative inside mdBook output", async () => {
  const {ready, loading} = bootstrap([createMount()], [], {
    pageUrl:"https://example.test/project/book/nested/chapter.html",
    currentScript:{dataset:{assetBase:"local-assets/"}},
    pathToRoot:"../",
  });
  await ready;
  assert.deepEqual(loading.fetches, ["https://example.test/project/book/nested/local-assets/adam-live-examples.json"]);
});

test("a failed graph dependency leaves a graph-free peer independently mounted", async () => {
  const first = createMount("a");
  const peer = createMount("b");
  const {ready, loading} = bootstrap([first, peer], [createMount("a")], {runtime:true, scriptError:true});
  await ready;
  assert.deepEqual(loading.mounts, ["adam-live-1"]);
  assert.equal(first.role, "alert");
  assert.equal(peer.role, undefined);
});

test("explicit root identities pair graphs independently of diagnostic names", () => {
  const { createMountPlan } = bootstrap();
  const plan = createMountPlan([{name:"a", root:"one"}, {name:"a", root:"two"}],
    [{name:"a", root:"two"}, {name:"a", root:"one"}], {a:"sheet a {}"});
  assert.deepEqual(Array.from(plan[0].graphIds), ["adam-live-graph-1"]);
  assert.deepEqual(Array.from(plan[1].graphIds), ["adam-live-graph-0"]);
});

test("mount plans reject missing sources, malformed manifests and orphan graphs", () => {
  const { createMountPlan } = bootstrap();
  for (const manifest of [null, [], { a: 12 }, {}]) {
    assert.throws(() => createMountPlan(["a"], [], manifest));
  }
  assert.throws(() => createMountPlan([], ["a"], { a: "sheet a {}" }));
  assert.throws(() => createMountPlan(["toString"], [], {}));
  assert.throws(() => createMountPlan([undefined], [], { undefined: "sheet a {}" }));
  assert.equal(createMountPlan([], [], {}).length, 0);
});

test("manifest HTTP failures reject before accepting a JSON body", async () => {
  const { readManifest } = bootstrap();
  await assert.rejects(readManifest(async () => ({ ok: false, status: 404, json: () => ({}) }), "manifest"));
  const manifest = { a: "sheet a {}" };
  assert.equal(await readManifest(async () => ({ ok: true, json: async () => manifest }), "manifest"), manifest);
});

test("load failures visibly identify every affected mount", () => {
  const { showLoadFailure } = bootstrap();
  const mounts = [createMount(), createMount()];
  showLoadFailure(mounts, new Error("offline"));
  for (const mount of mounts) {
    assert.match(mount.textContent, /offline/);
    assert.equal(mount.role, "alert");
  }
  showLoadFailure(mounts, null);
  for (const mount of mounts) assert.match(mount.textContent, /null/);
});

test("script dependencies include graph drivers only for graph pages", () => {
  const { dependencyScripts } = bootstrap();
  const plain = dependencyScripts(false);
  assert.equal(plain.length, 0);
  assert.equal(plain.some(item => item.url.endsWith("swc.js")), false);
  const graph = dependencyScripts(true);
  assert.deepEqual(Array.from(graph, (entry) => entry.url), [
    "https://example.test/book/theme/d3.v7.min.js",
    "https://example.test/book/theme/graph.js",
  ]);
  graph[0].url = "changed";
  assert.equal(dependencyScripts(true)[0].url, "https://example.test/book/theme/d3.v7.min.js");
});

test("script loading resolves on load, rejects on error, and distinguishes module scripts", async () => {
  const { document, loadScript } = bootstrap();
  document.createElement = () => ({});
  document.head = {
    // Checks the requested module descriptor before completing its load.
    appendChild(script) {
      assert.equal(script.src, "module.js");
      assert.equal(script.type, "module");
      script.onload();
    },
  };
  await loadScript("module.js", { module: true });
  document.head.appendChild = () => { throw new Error("duplicate dependency insertion"); };
  await loadScript("module.js", {module:true});
  document.head.appendChild = (script) => {
    assert.equal(script.src, "classic.js");
    assert.equal(script.type, undefined);
    script.onerror();
  };
  await assert.rejects(loadScript("classic.js"), /classic.js/);
});

test("page initialization does no loading without mounts and visibly handles dependency rejection", async () => {
  const empty = bootstrap();
  await empty.ready;
  assert.deepEqual(empty.loading, { fetches: [], scripts: [], imports: [], mounts: [] });
  const inspector = createMount();
  const graph = createMount();
  const page = bootstrap([inspector], [graph]);
  await page.ready;
  assert.equal(page.loading.fetches.length, 1);
  assert.equal(page.loading.scripts.length, 2);
  assert.equal(page.loading.imports.length, 1, "run Node with --experimental-vm-modules");
  assert.equal(inspector.role, "alert");
  assert.equal(graph.role, "alert");
  assert.match(inspector.textContent, /Live example unavailable/);
  assert.match(graph.textContent, /Live example unavailable/);
});
