import { spawnSync } from "node:child_process";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { renderDeck } from "../deck.mjs";

const input = resolve(process.env.SLIDES_INPUT_DIR ?? "dist");
const output = resolve(process.env.SLIDES_DIR ?? "test-results/site");
await mkdir(output, { recursive: true });
await cp(input, output, { recursive: true });
await cp(resolve(input, "index.html"), resolve(output, "authored.html"));
const manifestPath = resolve(output, "theme", "adam-live-examples.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
manifest["local/acceptance"] = manifest["tutorial/first_sheet"];
await writeFile(manifestPath, JSON.stringify(manifest));

const examples = [
  ["area_with_requirement", false], ["requirements_filter_diagnostic", false],
  ["conditional_forced", false], ["constrain", true], ["inequality", true],
  ["basic_relationship", true], ["basic_output", true], ["clamp_demo", false],
  ["first_sheet", true],
];
const authored = "---\nmarp: true\ntheme: adam-slides\nsize: 16:9\npaginate: true\n---\n\n"
  + "# Text-only introduction\n\nA presentation need not begin with an example.\n\n---\n\n"
  + examples.map(([name, graph]) => `# ${name === "first_sheet" ? "A first sheet" : name}\n\n`
    + `An interaction prompt.\n\nAdditional prose must leave room for the live pane.\n\n`
    + `<!-- adam-example: tutorial/${name} graph=${graph ? "1" : "0"} -->`).join("\n\n---\n\n")
  + "\n\n---\n\n# Deck-local example\n\n<!-- adam-example: local/acceptance -->";
await writeFile(resolve(output, "slides.md"), renderDeck(authored, (key) => {
  if (typeof manifest[key] !== "string") throw new Error(`Missing acceptance source: ${key}`);
  return manifest[key];
}));
const result = spawnSync(process.execPath, [
  fileURLToPath(new URL("../node_modules/@marp-team/marp-cli/marp-cli.js", import.meta.url)),
  resolve(output, "slides.md"), "--no-stdin", "--html", "--theme-set", "slides.css",
  "--output", resolve(output, "index.html"),
], { stdio: "inherit" });
if (result.error) throw result.error;
if (result.status !== 0) throw new Error(`Acceptance deck rendering failed: ${result.status}`);
