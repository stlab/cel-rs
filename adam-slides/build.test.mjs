import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { prepareDeck } from "./build.mjs";
import * as build from "./build.mjs";

/**
 * Creates an owned publication fixture with old output and new staged sources.
 * Complexity: O(n) in fixture bytes.
 */
async function publicationFixture() {
  const root = await mkdtemp(join(tmpdir(), "adam-slides-publication-"));
  const slides = join(root, "slides");
  const staged = join(root, "staged");
  await mkdir(join(slides, "dist", "theme"), { recursive: true });
  await mkdir(join(slides, "examples"));
  await mkdir(staged);
  await writeFile(join(slides, "dist", "slides.md"), "# Previous deck");
  await writeFile(join(slides, "dist", "theme", "adam-live-examples.json"),
    '{"local/custom":"sheet previous {}"}');
  for (const name of ["example.html", "example.css", "example.mjs"]) {
    await writeFile(join(slides, name), name);
  }
  for (const name of [
    "adam_lang_book_live.js", "adam_lang_book_live_bg.wasm", "swc.js",
    "inspector.css", "graph.css", "graph.js", "d3.v7.min.js",
  ]) await writeFile(join(staged, name), name);
  await writeFile(join(staged, "adam-live-examples.json"),
    '{"tutorial/first_sheet":"sheet book {}"}');
  await writeFile(join(slides, "examples", "custom.adm2"), "sheet current {}");
  return { root, slides, staged };
}

/** Catches replacing the last valid source/deck pair when a watched edit is invalid. */
test("source validation failures preserve the previously published deck and manifest", async () => {
  for (const [markdown, badName] of [
    ["<!-- adam-example: local/custom graph=true -->", false],
    ["<!-- adam-example: local/missing -->", false],
    ["<!-- adam-example: local/custom -->", true],
  ]) {
    const fixture = await publicationFixture();
    try {
      await writeFile(join(fixture.slides, "slides.md"), markdown);
      if (badName) await writeFile(join(fixture.slides, "examples", "Bad Name.adm2"), "sheet invalid {}");
      await assert.rejects(build.publishDeck(fixture.slides, fixture.staged), /directive|source|key/i);
      assert.equal(await readFile(join(fixture.slides, "dist", "slides.md"), "utf8"), "# Previous deck");
      assert.equal(await readFile(join(fixture.slides, "dist", "theme", "adam-live-examples.json"), "utf8"),
        '{"local/custom":"sheet previous {}"}');
    } finally {
      await rm(fixture.root, { recursive: true });
    }
  }
});

/** Catches publishing different source bytes to the deck and live inspector. */
test("successful publication uses merged sources for both displayed and mounted examples", async () => {
  const fixture = await publicationFixture();
  try {
    await writeFile(join(fixture.slides, "slides.md"),
      "<!-- adam-example: local/custom -->\n\n---\n\n<!-- adam-example: tutorial/first_sheet -->");
    await build.publishDeck(fixture.slides, fixture.staged);
    const output = join(fixture.slides, "dist");
    const manifest = JSON.parse(await readFile(join(output, "theme", "adam-live-examples.json"), "utf8"));
    const markdown = await readFile(join(output, "slides.md"), "utf8");
    assert.deepEqual(manifest, { "tutorial/first_sheet": "sheet book {}", "local/custom": "sheet current {}" });
    assert.ok(markdown.includes("sheet book {}\n```"));
    assert.ok(markdown.includes("sheet current {}\n```"));
    assert.equal(await readFile(join(output, "example.html"), "utf8"), "example.html");
    assert.equal(await readFile(join(output, "theme", "swc.js"), "utf8"), "swc.js");
  } finally {
    await rm(fixture.root, { recursive: true });
  }
});

/** Catches accepting stale output after a failed staging subprocess. */
test("failed asset staging rejects a build even when old output exists", async () => {
  const root = await mkdtemp(join(tmpdir(), "adam-slides-build-"));
  try {
    const output = join(root, "adam-slides", "dist");
    await mkdir(output, { recursive: true });
    await writeFile(join(output, "index.html"), "stale");
    await assert.rejects(prepareDeck(root), /staging|Cargo.toml/i);
    assert.equal(await readFile(join(output, "index.html"), "utf8"), "stale");
    await assert.rejects(readFile(join(output, "slides.md")), { code: "ENOENT" });
  } finally {
    await rm(root, { recursive: true });
  }
});

/** Catches losing local source bytes or flattening nested file identities. */
test("local examples load exact source bytes under reserved keys", async () => {
  const directory = await mkdtemp(join(tmpdir(), "adam-slides-local-"));
  try {
    await mkdir(join(directory, "nested"));
    await writeFile(join(directory, "custom.adm2"), "sheet custom {}\n");
    await writeFile(join(directory, "nested", "other.adm2"), "sheet other {}");
    await writeFile(join(directory, "README.txt"), "Not an example");
    assert.deepEqual(await build.readLocalExamples(directory), {
      "local/custom": "sheet custom {}\n",
      "local/nested/other": "sheet other {}",
    });
  } finally {
    await rm(directory, { recursive: true });
  }
});

/** Catches silently omitting invalid local source names or unreadable directories. */
test("local example discovery rejects unsafe names and missing directories", async () => {
  const directory = await mkdtemp(join(tmpdir(), "adam-slides-local-"));
  try {
    await writeFile(join(directory, "Bad Name.adm2"), "sheet bad {}");
    await assert.rejects(build.readLocalExamples(directory), /name|key|selector/i);
    await assert.rejects(build.readLocalExamples(join(directory, "missing")), { code: "ENOENT" });
  } finally {
    await rm(directory, { recursive: true });
  }
});
