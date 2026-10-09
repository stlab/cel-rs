import assert from "node:assert/strict";
import test from "node:test";
import { renderDeck } from "./deck.mjs";

/** Catches requiring examples on text-only slides, including an empty deck. */
test("text-only Markdown passes through unchanged", () => {
  for (const markdown of ["", "# Introduction\n\nText.\n\n---\n\n# Conclusion"]) {
    assert.equal(renderDeck(markdown, () => { throw new Error("Unexpected source read"); }), markdown);
  }
});

/** Catches enforcing tutorial order, membership, uniqueness, or slide count. */
test("mixed slides expand freely selected and repeated sources", () => {
  const markdown = "# Text\n\n---\n\n<!-- adam-example: local/custom graph=1 -->\n\n"
    + "---\n\n<!-- adam-example: expressions/arithmetic -->\n\n"
    + "<!-- adam-example: local/custom graph=0 -->";
  const keys = [];
  const result = renderDeck(markdown, (key) => {
    keys.push(key);
    return "sheet example {}";
  });
  assert.deepEqual(keys, ["local/custom", "expressions/arithmetic", "local/custom"]);
  assert.equal((result.match(/<iframe /g) ?? []).length, 3);
  assert.ok(result.includes("example=local%2Fcustom&amp;graph=1"));
  assert.ok(result.includes("example=expressions%2Farithmetic&amp;graph=0"));
  assert.ok(result.includes("example=local%2Fcustom&amp;graph=0"));
  assert.ok(result.startsWith("# Text\n\n---"));
});

/** Catches unsafe selectors and malformed or ambiguous authoring options. */
test("malformed directives fail before reading sources", () => {
  for (const directive of [
    "adam-example", "adam-example:", "adam-example: ../secret",
    "adam-example: tutorial/../../secret", "adam-example: /absolute",
    "adam-example: local/custom.adm2", "adam-example: local/custom graph=true",
    "adam-example: local/custom graph=1 graph=0", "adam-example: local/custom unknown=1",
  ]) {
    assert.throws(() => renderDeck(`<!-- ${directive} -->`,
      () => { throw new Error("Unexpected source read"); }), /directive|selector/i);
  }
});

/** Catches source truncation, broken fencing, and loss of frame selectors. */
test("rendering retains source safely and produces same-site frames", () => {
  const source = 'sheet x {\n// ``` <script>alert("x")</script>\n}\n';
  const result = renderDeck("<!-- adam-example: tutorial/first_sheet graph=1 -->", () => source);
  assert.ok(result.includes(`\`\`\`\`adam\n${source}\`\`\`\``));
  assert.ok(result.includes('src="example.html?example=tutorial%2Ffirst_sheet&amp;graph=1"'));
  assert.ok(result.includes('data-example="tutorial/first_sheet"'));
  assert.ok(result.includes('loading="eager"'));
  assert.ok(result.includes('title="Live Adam example: tutorial/first_sheet"'));
});

/** Catches swallowed source errors and missing trailing source newlines. */
test("source failures stop rendering and source fences terminate correctly", () => {
  const markdown = "<!-- adam-example: local/custom -->";
  assert.throws(() => renderDeck(markdown,
    () => { throw new Error("Missing source"); }), /Missing source/);
  assert.ok(renderDeck(markdown, () => "sheet x {}").includes("sheet x {}\n```"));
});
