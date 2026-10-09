/** @typedef {{ key: string, graph: boolean }} Example */

/**
 * Returns active tutorial examples in include order with their graph presence.
 * Throws for unsupported include references, duplicate examples, or unbound graphs.
 * Complexity: O(n) in Markdown length.
 * @param {string} markdown
 * @returns {Example[]}
 */
export function tutorialExamples(markdown) {
  const active = markdown.replace(/<!--[\s\S]*?-->/g, "");
  const examples = [];
  for (const match of active.matchAll(/\{\{#include\s+([^}]+)\}\}/g)) {
    const reference = match[1].trim();
    const key = /^examples\/(tutorial\/[a-z][a-z0-9_]*)\.adm2$/.exec(reference)?.[1];
    if (!key) throw new Error(`Unsupported tutorial include reference: ${reference}`);
    examples.push({ key, graph: false });
  }
  const byKey = new Map(examples.map((example) => [example.key, example]));
  if (byKey.size !== examples.length) throw new Error("Duplicate tutorial example");
  for (const match of active.matchAll(/<graph\s+sheet="([^"]+)"\s*>/g)) {
    const example = byKey.get(`tutorial/${match[1]}`);
    if (!example) throw new Error(`Graph has no tutorial example: ${match[1]}`);
    example.graph = true;
  }
  return examples;
}

/**
 * Expands one source/frame directive per slide after validating exact coverage.
 * Throws for invalid directives, coverage drift, or source read failures.
 * Complexity: O(n + s) in Markdown and included source bytes.
 * @param {string} markdown
 * @param {Example[]} examples
 * @param {(key: string) => string} readSource
 * @returns {string}
 */
export function renderDeck(markdown, examples, readSource) {
  const body = markdown.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "");
  const slides = body.split(/^---\s*$/m);
  if (slides.length !== examples.length) throw new Error("Slide count differs from tutorial example count");
  const directives = [];
  for (const slide of slides) {
    const matches = [...slide.matchAll(/<!--\s*adam-example\b([\s\S]*?)-->/g)];
    if (matches.length !== 1) throw new Error("Each slide needs exactly one example directive");
    const key = /^:\s*(tutorial\/[a-z][a-z0-9_]*)\s*$/.exec(matches[0][1])?.[1];
    if (!key) throw new Error("Invalid example directive");
    directives.push({ key });
  }
  for (const [index, directive] of directives.entries()) {
    if (directive.key !== examples[index].key) throw new Error(`Example order or coverage differs: ${directive.key}`);
  }
  let next = 0;
  return markdown.replace(/<!--\s*adam-example\b[\s\S]*?-->/g, () => {
    const { key, graph } = examples[next++];
    const source = readSource(key);
    let longest = 2;
    for (const match of source.matchAll(/`+/g)) longest = Math.max(longest, match[0].length);
    const fence = "`".repeat(longest + 1);
    const terminated = source.endsWith("\n") ? source : `${source}\n`;
    const src = `example.html?example=${encodeURIComponent(key)}&amp;graph=${graph ? "1" : "0"}`;
    return `<div class="example-panes">\n\n${fence}adam\n${terminated}${fence}\n\n`
      + `<iframe src="${src}" data-example="${key}" title="Live Adam example: ${key}" loading="eager"></iframe>\n\n</div>`;
  });
}
