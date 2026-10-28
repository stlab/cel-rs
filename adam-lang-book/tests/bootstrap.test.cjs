// Exercises the authored bootstrap's decisions without a browser or external dependencies.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const vm = require("node:vm");

// Loads the real classic script with a caller-owned page and blocked external module loading.
// Time and space follow script evaluation and the supplied page's mount counts.
function bootstrap(inspectors = [], graphs = []) {
  const context = vm.createContext({
    URL,
    document: {
      baseURI: "https://example.test/book/chapter.html",
      querySelectorAll: (selector) => selector === ".adam-live" ? inspectors : graphs,
      createElement: () => ({}),
      head: {
        // Completes a caller-supplied script load synchronously.
        appendChild(script) { script.onload(); },
      },
    },
    fetch: async () => ({ ok: false, status: 503 }),
    console: {
      // Discards expected page-level errors in the isolated test context.
      error() {},
    },
  });
  context.ready = vm.runInContext(
    readFileSync(join(__dirname, "../book-src/theme/adam-live-bootstrap.js"), "utf8"),
    context,
    { importModuleDynamically: async () => { throw new Error("blocked module"); } },
  );
  return vm.runInContext("({ createMountPlan, readManifest, showLoadFailure, dependencyScripts, loadScript, document, ready })", context);
}

// Creates an independent caller-owned DOM stand-in with visible attribute and text state.
function createMount(example = "a") {
  return {
    dataset: { example },
    // Records an attribute on this mount, never on another fixture.
    setAttribute(name, value) { this[name] = value; },
  };
}

test("mount plans preserve shared graph order and own independent graph lists", () => {
  const { createMountPlan } = bootstrap();
  const plan = createMountPlan(["a", "a", "b"], ["a", "b", "a"], { a: "sheet a {}", b: "sheet b {}" });
  assert.deepEqual(Array.from(plan[0].graphIds), ["adam-live-graph-0", "adam-live-graph-2"]);
  assert.deepEqual(Array.from(plan[2].graphIds), ["adam-live-graph-1"]);
  plan[0].graphIds.push("changed");
  assert.equal(plan[1].graphIds.length, 2);
  plan[1].graphIds.splice(0, 1);
  assert.equal(plan[0].graphIds.length, 3);
  assert.equal(plan[0].source, "sheet a {}");
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
  assert.equal(plain.length, 1);
  assert.equal(plain[0].url, "https://example.test/book/theme/swc.js");
  assert.equal(plain[0].module, true);
  const graph = dependencyScripts(true);
  assert.deepEqual(Array.from(graph, (entry) => entry.url), [
    "https://example.test/book/theme/swc.js",
    "https://example.test/book/theme/d3.v7.min.js",
    "https://example.test/book/theme/graph.js",
  ]);
  plain[0].url = "changed";
  assert.equal(dependencyScripts(false)[0].url, "https://example.test/book/theme/swc.js");
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
  const inspector = createMount();
  const graph = createMount();
  const page = bootstrap([inspector], [graph]);
  await page.ready;
  assert.equal(inspector.role, "alert");
  assert.equal(graph.role, "alert");
  assert.match(inspector.textContent, /Live example unavailable/);
  assert.match(graph.textContent, /Live example unavailable/);
});
