/** Verifies the shared numeric input bridge through its observable payload contract. */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../adam-web-ui/src/numeric_input.js", import.meta.url), "utf8");
const context = vm.createContext({ Intl });
vm.runInContext(source, context);

/** Creates the DOM boundary needed to read a field without emulating its numeric formatting. */
function field(raw, locale) {
    return {
        numberFormatter: new Intl.NumberFormat(locale),
        shadowRoot: { querySelector: () => ({ value: raw }) },
    };
}

/** Creates a slider with a real locale formatter and optional localized draft text. */
function slider(value, locale, draft) {
    const host = { value, shadowRoot: { querySelector: () => field("", locale) } };
    if (draft !== undefined) host.adamNumericDraft = draft;
    return host;
}

test("slider drags preserve canonical fractional values across locales", () => {
    for (const locale of ["en-US", "de-DE", "es-ES", "fr-FR"]) {
        for (const value of [3.6, 12.25, -1.75]) {
            const host = slider(value, locale);
            assert.equal(context.readNumericInput(host, true, false), `0|${value}`);
        }
    }
});

test("slider inline drafts normalize localized separators and are consumed once", () => {
    for (const [locale, draft] of [["de-DE", "1.234,50"], ["en-US", "1,234.50"]]) {
        const host = slider(12.25, locale, draft);
        assert.equal(context.readNumericInput(host, true, true), "1|1234.50");
        assert.equal(Object.hasOwn(host, "adamNumericDraft"), false);
        assert.equal(context.readNumericInput(host, true, false), "0|12.25");
    }
});

test("blank slider drafts remain blank while the handle retains its value", () => {
    const host = slider(51, "de-DE", "");
    assert.equal(context.readNumericInput(host, true, true), "1|");
    assert.equal(host.value, 51);
});

test("number fields preserve partial and blank drafts while normalizing locale text", () => {
    for (const [raw, expected] of [["1.234,50", "1234.50"], ["1,", "1."], ["", ""]]) {
        assert.equal(context.readNumericInput(field(raw, "de-DE"), false, true), `1|${expected}`);
    }
});

/** Supplies event boundaries and an explicit next-task queue for exercising the installed bridge. */
function mountedSlider(raw, value) {
    const scheduled = [];
    const listeners = new Map();
    const events = [];
    const inline = field(raw, "de-DE");
    inline.value = value;
    const host = {
        value: 51,
        isConnected: true,
        shadowRoot: { querySelector: () => inline },
        addEventListener: (name, listener, capture) => {
            assert.equal(capture, true);
            listeners.set(name, listener);
        },
        dispatchEvent: event => events.push(event),
    };
    const scope = vm.createContext({
        setTimeout: callback => scheduled.push(callback),
        Event: class {
            /** Retains the event's observable type and options. */
            constructor(type, options) {
                this.type = type;
                Object.assign(this, options);
            }
        },
    });
    vm.runInContext(source, scope);
    scope.installNumericSliderBridge(host);
    return { host, inline, scheduled, listeners, events };
}

test("slider bridge forwards parsed inline values and localized drafts on the next task", () => {
    const fixture = mountedSlider("3,60", 3.6);
    fixture.listeners.get("input")({ composedPath: () => [fixture.inline, fixture.host] });
    assert.equal(fixture.events.length, 0);
    fixture.scheduled[0]();
    assert.equal(fixture.host.value, 3.6);
    assert.equal(fixture.host.adamNumericDraft, "3,60");
    assert.equal(fixture.events.length, 1);
    assert.equal(fixture.events[0].type, "input");
    assert.equal(fixture.events[0].bubbles, true);
    assert.equal(fixture.events[0].composed, true);
});

test("slider bridge preserves the handle for blank drafts and suppresses empty inline changes", () => {
    const fixture = mountedSlider("", NaN);
    fixture.listeners.get("input")({ composedPath: () => [fixture.inline] });
    fixture.scheduled[0]();
    assert.equal(fixture.host.value, 51);
    assert.equal(context.readNumericInput(fixture.host, true, true), "1|");
    let stopped = false;
    fixture.listeners.get("change")({
        composedPath: () => [fixture.inline],
        stopImmediatePropagation: () => { stopped = true; },
    });
    assert.equal(stopped, true);
});

test("slider bridge ignores handle events, managed fields, and disconnected callbacks", () => {
    const fixture = mountedSlider("3,60", 3.6);
    fixture.listeners.get("input")({ composedPath: () => [fixture.host] });
    assert.equal(fixture.scheduled.length, 0);
    fixture.inline.managedInput = true;
    fixture.listeners.get("input")({ composedPath: () => [fixture.inline] });
    assert.equal(fixture.scheduled.length, 0);
    fixture.inline.managedInput = false;
    fixture.listeners.get("input")({ composedPath: () => [fixture.inline] });
    fixture.host.isConnected = false;
    fixture.scheduled[0]();
    assert.equal(fixture.host.value, 51);
    assert.equal(fixture.events.length, 0);
});

test("slider bridge leaves nonempty inline and handle change events untouched", () => {
    const fixture = mountedSlider("3,60", 3.6);
    for (const path of [[fixture.inline], [fixture.host]]) {
        fixture.listeners.get("change")({
            composedPath: () => path,
            stopImmediatePropagation: () => assert.fail("nonempty changes must not be suppressed"),
        });
    }
});
