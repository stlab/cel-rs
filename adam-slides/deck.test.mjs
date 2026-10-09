import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderDeck, tutorialExamples } from "./deck.mjs";

const keys = [
  "first_sheet", "clamp_demo", "basic_output", "basic_relationship",
  "inequality", "constrain", "conditional_forced",
  "requirements_filter_diagnostic", "area_with_requirement",
].map((name) => `tutorial/${name}`);

/** Builds authored slide directives for a hand-selected ordered key list. */
function deck(names) {
  return names.map((key) => `# Example\n\n<!-- adam-example: ${key} -->`).join("\n\n---\n\n");
}

/** Catches accidental inclusion of hidden examples and incorrect graph pairing. */
test("tutorial examples preserve the nine active includes and graph presence", () => {
  const tutorial = readFileSync(new URL("../adam-lang-book/book-src/tutorial.md", import.meta.url), "utf8");
  const examples = tutorialExamples(tutorial);
  assert.deepEqual(examples.map((example) => example.key), keys);
  assert.deepEqual(examples.map((example) => example.graph),
    [true, false, true, true, true, true, false, false, false]);
});

/** Catches unrecognized graph declarations and duplicate active includes. */
test("tutorial validation rejects graphs without examples and duplicate examples", () => {
  assert.throws(() => tutorialExamples('<graph sheet="missing">'), /graph/i);
  const include = "{{#include examples/tutorial/first_sheet.adm2}}";
  assert.throws(() => tutorialExamples(`${include}\n${include}`), /duplicate/i);
});

/** Catches active source references silently disappearing from slide coverage. */
test("tutorial validation rejects unsupported includes mixed with valid examples", () => {
  const first = "{{#include examples/tutorial/first_sheet.adm2}}";
  for (const reference of [
    "examples/tutorial/missing-sheet.adm2",
    "examples/tutorial/first_sheet.adm2:1:4",
    "examples/tutorial/../secret.adm2",
    "examples/other/chapter.adm2",
  ]) {
    assert.throws(() => tutorialExamples(`${first}\n{{#include ${reference}}}`), /include|reference/i);
  }
});

/** Catches key changes, missing directives, duplicate slides, and unsafe references. */
test("deck validation rejects coverage drift and malformed directives", () => {
  const examples = keys.map((key) => ({ key, graph: false }));
  for (const invalid of [
    deck(keys.slice(1)),
    deck([...keys, keys[0]]),
    deck([...keys].reverse()),
    deck([...keys.slice(0, -1), "tutorial/unknown"]),
    deck(keys).replace(keys[0], "tutorial/../../secret"),
    deck(keys).replace("adam-example:", "adam-example"),
    deck(keys).replace("<!-- adam-example: tutorial/first_sheet -->", ""),
    deck(keys).replace("\n\n---\n\n", "\n\n"),
  ]) {
    assert.throws(() => renderDeck(invalid, examples, () => "sheet x {}"), /example|slide|directive/i);
  }
});

/** Catches source truncation, broken fencing, and loss of frame selectors. */
test("rendering retains canonical source safely and produces same-site frames", () => {
  const source = 'sheet x {\n// ``` <script>alert("x")</script>\n}\n';
  const result = renderDeck(deck(["tutorial/first_sheet"]),
    [{ key: "tutorial/first_sheet", graph: true }], () => source);
  assert.ok(result.includes(`\`\`\`\`adam\n${source}\`\`\`\``));
  assert.ok(result.includes('src="example.html?example=tutorial%2Ffirst_sheet&amp;graph=1"'));
  assert.ok(result.includes('data-example="tutorial/first_sheet"'));
  assert.ok(result.includes('loading="eager"'));
  assert.ok(result.includes('title="Live Adam example: tutorial/first_sheet"'));
});

/** Catches swallowed filesystem failures while resolving canonical source. */
test("source read failures stop rendering", () => {
  assert.throws(() => renderDeck(deck(["tutorial/first_sheet"]),
    [{ key: "tutorial/first_sheet", graph: false }],
    () => { throw new Error("Missing canonical source"); }), /Missing canonical source/);
});
