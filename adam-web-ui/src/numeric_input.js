/**
 * Returns a focus-prefixed numeric input payload from a Spectrum field or editable slider.
 * Consumes and deletes a slider's localized `adamNumericDraft`; otherwise reads its numeric handle.
 * Preserves canonical handle numbers across locales, and preserves blank and partial localized drafts.
 * Runtime: O(n) in the draft text length.
 */
function readNumericInput(host, slider, focused) {
    const draft = host.adamNumericDraft;
    delete host.adamNumericDraft;
    if (slider && draft === undefined) {
        return (focused ? "1" : "0") + "|" + host.value.toString();
    }
    const field = slider ? host.shadowRoot.querySelector("sp-number-field") : host;
    let raw = slider
        ? draft
        : field.shadowRoot.querySelector("input").value;
    let group = "", decimal = ".";
    for (const part of field.numberFormatter.formatToParts(1234.5)) {
        if (part.type === "group") group = part.value;
        if (part.type === "decimal") decimal = part.value;
    }
    if (group) raw = raw.split(group).join("");
    if (decimal !== ".") raw = raw.split(decimal).join(".");
    return (focused ? "1" : "0") + "|" + raw;
}

/**
 * Installs capture listeners once on a mounted Spectrum slider.
 * After Spectrum parses an unmanaged inline input, retains its raw localized text (including blanks)
 * in `adamNumericDraft` for `readNumericInput` to consume, then redispatches a host input event.
 * Updates the handle only for finite inline values and skips callbacks after unmount.
 * Suppresses empty inline change events so Spectrum does not restore the previous value on blur.
 */
function installNumericSliderBridge(host) {
    host.addEventListener("input", event => {
        const field = host.shadowRoot.querySelector("sp-number-field");
        if (!field || !event.composedPath().includes(field) || field.managedInput) return;
        // Spectrum parses the original event before this next-task callback reads the field.
        setTimeout(() => {
            if (!host.isConnected) return;
            if (Number.isFinite(field.value)) host.value = field.value;
            host.adamNumericDraft = field.shadowRoot.querySelector("input").value;
            host.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
        }, 0);
    }, true);
    host.addEventListener("change", event => {
        const field = host.shadowRoot.querySelector("sp-number-field");
        if (field && event.composedPath().includes(field)
            && field.shadowRoot.querySelector("input").value === "") {
            event.stopImmediatePropagation();
        }
    }, true);
}
