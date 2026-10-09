import { isExampleKey } from "./example.mjs";

/**
 * Expands example directives without constraining slide count, selection, or order.
 * Graphs default to disabled; `graph=1` enables a directive's graph.
 * Throws for malformed directives, unsafe keys, or source read failures.
 * Complexity: O(n + s) in Markdown and included source bytes.
 * @param {string} markdown
 * @param {(key: string) => string} readSource
 * @returns {string}
 */
export function renderDeck(markdown, readSource) {
  return markdown.replace(/<!--\s*adam-example\b([\s\S]*?)-->/g, (_, directive) => {
    const options = /^:\s*(\S+)(?:\s+graph=([01]))?\s*$/.exec(directive);
    if (!options || !isExampleKey(options[1])) throw new Error(`Invalid example directive: ${directive}`);
    const key = options[1];
    const graph = options[2] === "1";
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
