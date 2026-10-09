import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { access, copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { renderDeck, tutorialExamples } from "./deck.mjs";

/**
 * Stages runtime assets and host files, then generates canonical Marp Markdown.
 * Rejects staging, source validation, or filesystem failures.
 * Complexity: O(n) in source and staged asset bytes.
 * @param {string} root Repository root.
 * @returns {Promise<void>}
 */
export async function prepareDeck(root) {
  const result = spawnSync("cargo", ["run", "-p", "xtask", "--", "prepare-live-slides-assets"],
    { cwd: root, encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Live asset staging failed:\n${result.stderr}`);
  process.stdout.write(result.stdout);
  const slides = join(root, "adam-slides");
  const output = join(slides, "dist");
  for (const asset of [
    "adam-live-examples.json", "adam_lang_book_live.js", "adam_lang_book_live_bg.wasm",
    "swc.js", "inspector.css", "graph.css", "graph.js", "d3.v7.min.js",
  ]) await access(join(output, "theme", asset));
  const tutorial = await readFile(join(root, "adam-lang-book", "book-src", "tutorial.md"), "utf8");
  const authored = await readFile(join(slides, "slides.md"), "utf8");
  const markdown = renderDeck(authored, tutorialExamples(tutorial),
    (key) => readFileSync(join(root, "adam-lang-book", "book-src", "examples", `${key}.adm2`), "utf8"));
  await mkdir(output, { recursive: true });
  for (const name of ["example.html", "example.css", "example.mjs"]) {
    await copyFile(join(slides, name), join(output, name));
  }
  await writeFile(join(output, "slides.md"), markdown);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await prepareDeck(fileURLToPath(new URL("../", import.meta.url)));
}
