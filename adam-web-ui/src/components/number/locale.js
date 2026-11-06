/**
 * Resolves decimal symbols using Intl parts, without reading DOM state.
 * Returns a deeply frozen vocabulary; throws for invalid/unsupported locales.
 * Runtime and space: O(locale identifier bytes), with fixed-size samples.
 */
export function numberLocale(locale) {
    const resolved = resolvedLocale(locale);
    const formatter = new Intl.NumberFormat(resolved);
    const parts = formatter.formatToParts(-1234567890123.5);
    const plusParts = new Intl.NumberFormat(resolved, { signDisplay: "always" }).formatToParts(1);
    const groups = parts.filter(part => part.type === "integer").map(part => [...part.value].length);
    const digits = Array.from({ length: 10 }, (_, digit) =>
        formatter.formatToParts(digit).find(part => part.type === "integer").value);
    return Object.freeze({
        resolved,
        digits: Object.freeze(digits),
        decimal: parts.find(part => part.type === "decimal").value,
        group: parts.find(part => part.type === "group")?.value ?? "",
        minus: parts.find(part => part.type === "minusSign").value,
        plus: plusParts.find(part => part.type === "plusSign").value,
        ignorable: Object.freeze(["\u061c", "\u200e", "\u200f"]),
        primary_group: groups.at(-1),
        secondary_group: groups.at(-2) ?? groups.at(-1),
    });
}

/**
 * Formats validated canonical text with caller-selected Intl options.
 * Integer text passes directly to BigInt, never to Number. Float text must
 * already satisfy the Rust concrete-type range check; this boundary also rejects
 * malformed/nonfinite f64 input. Model NaN/infinity display bypasses this helper.
 * The default preserves float precision; explicit digit options select rounding.
 * Throws for invalid input, unsupported locales, or invalid Intl options.
 * Runtime and space: O(canonical bytes), excluding Intl internals.
 */
export function formatNumber(canonical, integer, locale, options = {}) {
    if (typeof canonical !== "string" || !(integer
        ? /^[+-]?\d+$/
        : /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/).test(canonical)) {
        throw new TypeError("Invalid canonical numeric text");
    }
    const value = integer ? BigInt(canonical) : Number(canonical);
    if (!integer && !Number.isFinite(value)) throw new RangeError("Nonfinite numeric text");
    const precision = !integer && !Object.keys(options).some(key => key.endsWith("Digits"))
        ? { maximumSignificantDigits: 21 } : {};
    return new Intl.NumberFormat(resolvedLocale(locale), { ...precision, ...options }).format(value);
}

/**
 * Resolves a supported locale, or the host default when omitted.
 * Throws instead of silently substituting an unsupported request.
 * Runtime and space: O(locale identifier bytes), excluding Intl internals.
 */
function resolvedLocale(locale) {
    if (locale !== undefined) {
        if (typeof locale !== "string" || Intl.NumberFormat.supportedLocalesOf([locale]).length !== 1) {
            throw new RangeError("Unsupported numeric locale");
        }
    }
    return new Intl.NumberFormat(locale).resolvedOptions().locale;
}
