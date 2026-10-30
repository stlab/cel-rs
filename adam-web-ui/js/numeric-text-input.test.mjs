import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

/** Evaluates the actual shared reader against a text-valued control with inherited language. */
function reader(locale) {
  const source = readFileSync(new URL("../src/numeric_input.js", import.meta.url), "utf8");
  return vm.runInNewContext(source + "\nreadNumericInput", {
    Intl,
    document: { documentElement: { lang: locale } },
  });
}

test("exact text controls normalize inherited-locale integer drafts without Number coercion", () => {
  for (const [locale, text, expected] of [
    ["en-US", "9,007,199,254,740,997", "9007199254740997"],
    ["de-DE", "-9.223.372.036.854.775.808", "-9223372036854775808"],
    ["en-US", "18,446,744,073,709,551,615", "18446744073709551615"],
  ]) {
    const host = {
      value: text,
      closest: () => null,
      shadowRoot: { querySelector: () => ({ value: text }) },
    };
    assert.equal(reader(locale)(host, false, true), `1|${expected}`);
    assert.equal(host.value, text);
  }
});
