import assert from "node:assert/strict";
import test from "node:test";
import { exampleAssetUrls, parseExampleOptions } from "./example.mjs";

/** Catches implicit defaults and ambiguous or unsafe example selectors. */
test("example selectors require one safe key and an explicit graph flag", () => {
  const base = "https://example.test/project/docs/adam-slides/example.html";
  assert.deepEqual(parseExampleOptions(new URL(`${base}?example=tutorial%2Ffirst_sheet&graph=1`)),
    { key: "tutorial/first_sheet", graph: true });
  assert.deepEqual(parseExampleOptions(new URL(`${base}?example=tutorial%2Fclamp_demo&graph=0`)),
    { key: "tutorial/clamp_demo", graph: false });
  for (const query of [
    "", "?example=tutorial/first_sheet", "?graph=0",
    "?example=../secret&graph=0", "?example=tutorial/first_sheet&graph=true",
    "?example=tutorial/first_sheet&graph=0&graph=1",
    "?example=tutorial/first_sheet&example=tutorial/clamp_demo&graph=0",
  ]) {
    assert.throws(() => parseExampleOptions(new URL(base + query)), /example|graph/i);
  }
});

/** Catches root-relative URLs that break GitHub Pages project deployments. */
test("asset URLs preserve a nested static hosting path", () => {
  const urls = exampleAssetUrls(new URL("https://example.test/project/docs/adam-slides/example.html?example=x"));
  assert.deepEqual(Object.fromEntries(Object.entries(urls).map(([key, value]) => [key, value.href])), {
    module: "https://example.test/project/docs/adam-slides/theme/adam_lang_book_live.js",
    manifest: "https://example.test/project/docs/adam-slides/theme/adam-live-examples.json",
    spectrum: "https://example.test/project/docs/adam-slides/theme/swc.js",
    d3: "https://example.test/project/docs/adam-slides/theme/d3.v7.min.js",
    graph: "https://example.test/project/docs/adam-slides/theme/graph.js",
  });
});
