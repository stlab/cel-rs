import { spawnSync } from "node:child_process";
import { access, copyFile, cp, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { renderDeck } from "./deck.mjs";
import { isExampleKey } from "./example.mjs";

/**
 * Reads deck-local `.adm2` files recursively into a `local/`-prefixed source map.
 * Ignores other file extensions; rejects unsafe names and filesystem failures.
 * Complexity: O(n) in directory entries and source bytes.
 * @param {string} directory
 * @returns {Promise<Record<string, string>>}
 */
export async function readLocalExamples(directory) {
  const sources = {};
  for (const entry of await readdir(directory, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(".adm2")) continue;
    const file = join(entry.parentPath, entry.name);
    const key = `local/${relative(directory, file).slice(0, -5).split(sep).join("/")}`;
    if (!isExampleKey(key)) throw new Error(`Invalid local example key: ${key}`);
    sources[key] = await readFile(file, "utf8");
  }
  return sources;
}

/**
 * Validates sources before publishing staged runtime assets and generated Markdown.
 * Source validation failures leave the previous published manifest and Markdown unchanged.
 * Rejects missing assets, source validation errors, and filesystem failures.
 * Complexity: O(n) in source and asset bytes.
 * @param {string} slides Slide authoring directory.
 * @param {string} staged Runtime asset directory owned by the caller.
 * @returns {Promise<void>}
 */
export async function publishDeck(slides, staged) {
  const output = join(slides, "dist");
  for (const asset of [
    "adam-live-examples.json", "adam_lang_book_live.js", "adam_lang_book_live_bg.wasm",
    "swc.js", "inspector.css", "graph.css", "graph.js", "d3.v7.min.js",
  ]) await access(join(staged, asset));
  for (const name of ["example.html", "example.css", "example.mjs"]) {
    await access(join(slides, name));
  }
  const manifestPath = join(staged, "adam-live-examples.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  if (Object.keys(manifest).some((key) => key.startsWith("local/"))) {
    throw new Error("Book examples cannot use the reserved local/ namespace");
  }
  Object.assign(manifest, await readLocalExamples(join(slides, "examples")));
  const authored = await readFile(join(slides, "slides.md"), "utf8");
  const markdown = renderDeck(authored, (key) => {
    if (!Object.hasOwn(manifest, key) || typeof manifest[key] !== "string") {
      throw new Error(`Unknown or unsupported example source: ${key}`);
    }
    return manifest[key];
  });
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2));
  await mkdir(output, { recursive: true });
  for (const name of ["example.html", "example.css", "example.mjs"]) {
    await copyFile(join(slides, name), join(output, name));
  }
  await cp(staged, join(output, "theme"), { recursive: true });
  await writeFile(join(output, "slides.md"), markdown);
  console.log("Adam slides: source generation complete");
}

/**
 * Stages assets privately, validates and publishes the deck, and removes staging.
 * Rejects staging, source validation, publication, or cleanup failures.
 * Complexity: O(n) in source and staged asset bytes.
 * @param {string} root Repository root.
 * @returns {Promise<void>}
 */
export async function prepareDeck(root) {
  const staging = await mkdtemp(join(tmpdir(), "adam-slides-build-"));
  try {
    const result = spawnSync("cargo", ["run", "-p", "xtask", "--", "prepare-live-slides-assets", staging],
      { cwd: root, encoding: "utf8" });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`Live asset staging failed:\n${result.stderr}`);
    process.stdout.write(result.stdout);
    await publishDeck(join(root, "adam-slides"), staging);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await prepareDeck(fileURLToPath(new URL("../", import.meta.url)));
}
