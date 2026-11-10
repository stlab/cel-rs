import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../src/components/number/locale.js", import.meta.url), "utf8");
const { numberLocale, formatNumber } = await import(`data:text/javascript,${encodeURIComponent(source)}`);

test("resolves symbols and formats en-US/de-DE/fr-FR/ar-EG", () => {
    for (const [locale, decimal, group, formatted] of [
        ["en-US", ".", ",", "1,234.5"],
        ["de-DE", ",", ".", "1.234,5"],
        ["fr-FR", ",", "\u202f", "1\u202f234,5"],
        ["ar-EG", "\u066b", "\u066c", "١٬٢٣٤٫٥"],
    ]) {
        const symbols = numberLocale(locale);
        assert.equal(symbols.decimal, decimal);
        assert.equal(symbols.group, group);
        assert.equal(symbols.minus, "-");
        assert.equal(symbols.plus, "+");
        assert.equal(symbols.digits.length, 10);
        assert.equal(formatNumber("1234.5", false, locale), formatted);
    }
    assert.equal(numberLocale("ar-EG").digits[9], "٩");
    assert.ok(numberLocale("ar-EG").ignorable.includes("\u061c"));
    assert.equal(numberLocale("hi-IN").secondary_group, 2);
});

test("resolves non-ASCII Swedish minus and formats both signed values", () => {
    const symbols = numberLocale("sv-SE");
    assert.equal(symbols.minus, "−");
    assert.equal(symbols.plus, "+");
    assert.equal(formatNumber("-12.5", false, "sv-SE"), "−12,5");
    assert.equal(formatNumber("12.5", false, "sv-SE", { signDisplay: "always" }), "+12,5");
});

test("formats full-width integers without Number coercion", () => {
    assert.equal(formatNumber("18446744073709551615", true, "en-US"), "18,446,744,073,709,551,615");
    assert.equal(formatNumber("340282366920938463463374607431768211455", true, "en-US", { useGrouping: false }), "340282366920938463463374607431768211455");
    assert.equal(formatNumber("-170141183460469231731687303715884105728", true, "en-US", { useGrouping: false }), "-170141183460469231731687303715884105728");
});

test("rejects malformed, nonfinite, overflowing and unsupported locale input explicitly", () => {
    for (const text of ["", " ", "NaN", "Infinity", "1e9999", "0x10", "1x"]) {
        assert.throws(() => formatNumber(text, false, "en-US"));
    }
    assert.throws(() => formatNumber("1.2", true, "en-US"));
    assert.throws(() => numberLocale("not_a_locale"));
    assert.throws(() => numberLocale("zz-ZZ"));
    assert.equal(formatNumber("1.25", false, "de-DE", { minimumFractionDigits: 2 }), "1,25");
});

test("vocabulary cannot be mutated through returned nested arrays", () => {
    const symbols = numberLocale("en-US");
    assert.throws(() => { symbols.digits[1] = "X"; }, TypeError);
    assert.throws(() => { symbols.ignorable.push("x"); }, TypeError);
    assert.throws(() => { symbols.decimal = "X"; }, TypeError);
    assert.equal(numberLocale("en-US").digits[1], "1");
    assert.throws(() => formatNumber("1", false, "en-US", { minimumFractionDigits: -1 }));
});

test("default float display does not round finite fractional or subnormal model values to another value", () => {
    assert.equal(formatNumber("1.2345678901234567", false, "en-US"), "1.2345678901234567");
    const subnormal = formatNumber("5e-324", false, "en-US", { useGrouping: false });
    assert.equal(Number(subnormal), 5e-324);
});
