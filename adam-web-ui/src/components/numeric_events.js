/**
 * Captures one native text input synchronously and sends immutable JSON frames.
 * Validates actual full snapshots against the composition's selected interval;
 * never constructs a submitted value from a composed passage. Time/space O(n)
 * per event in text length. Missing authoritative ranges report a limitation.
 */
export function createNumericSource(element, token, send) {
    let epoch = "0", sequence = 0n, sessionId = 0n;
    let session = null, fence = null, stopped = false;
    let lastValue = element.value;
    let lastSelection = [element.selectionStart,element.selectionEnd,element.selectionDirection];
    const listeners = [];
    let resolveClosed;
    const closed = new Promise(resolve => { resolveClosed = resolve; });

    /** Captures only this element's current source value and UTF-16 selection. */
    function snapshot(event = {}) {
        lastValue = element.value;
        lastSelection = [element.selectionStart,element.selectionEnd,element.selectionDirection];
        return {
            value: lastValue, start: element.selectionStart, end: element.selectionEnd,
            direction: element.selectionDirection, data: event.data ?? null,
            input_type: event.inputType ?? null, composing: event.isComposing ?? null,
            native_type: event.type ?? null,
        };
    }

    /** Sends a complete snapshot stamped in the source's ordering domain. */
    function emit(action, event, extra = {}) {
        if (stopped) return;
        send({token, epoch, sequence: String(++sequence), session: String(sessionId),
            action, ...(event ? snapshot(event) : {value:lastValue}), ...extra});
    }

    /** Removes every listener owned by this input, including after setup failure. */
    function close() {
        if (stopped) return;
        stopped = true;
        for (const [type, listener] of listeners) element.removeEventListener(type, listener, true);
        session = fence = null;
        resolveClosed();
    }

    /** Retires ambiguous composition without treating it as malformed numeric text. */
    function limitation(message, event) {
        session = fence = null;
        emit("cancel", event);
        emit("error", event, {message: "Native composition range limitation: " + message});
    }

    /** Returns the captured middle only when both anchored outside spans match. */
    function middle(value) {
        if (!session || value.length < session.prefix.length + session.suffix.length
            || !value.startsWith(session.prefix) || !value.endsWith(session.suffix)) return null;
        return value.slice(session.prefix.length, value.length - session.suffix.length);
    }

    /** Recognizes native composition mutation metadata, not ordinary equal-valued edits. */
    function terminal(event) {
        return ["insertCompositionText", "insertFromComposition", "deleteCompositionText"]
            .includes(event.inputType);
    }

    /** Restores the last accepted snapshot synchronously after a retired native mutation. */
    function restore() {
        if (element.value === lastValue) return;
        element.value = lastValue;
        if (lastSelection[0] !== null && lastSelection[1] !== null) {
            element.setSelectionRange(...lastSelection);
        }
    }

    /** Completes only an actual captured full snapshot matching the final owned passage. */
    function complete(event) {
        if (!session?.ended || middle(element.value) !== session.final) return false;
        if (session.final === "" && element.value === session.initial) return false;
        fence = {value: element.value, data: session.final};
        session = null;
        emit("end", event);
        return true;
    }

    /** Starts a new exact native selection interval, including a collapsed insertion. */
    function start(event) {
        const {value, start, end} = snapshot(event);
        fence = null;
        sessionId++;
        if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end < start || end > value.length) {
            limitation("text selection is unavailable", event);
            return;
        }
        session = {prefix: value.slice(0,start), suffix: value.slice(end), initial: value, ended:false};
        emit("start", event);
    }

    /** Expands an owned span only from a synchronous authoritative pre-mutation selection. */
    function before(event) {
        if (!event.isComposing && !terminal(event)) {
            if (session) emit("cancel", event);
            session = fence = null;
            return;
        }
        if (!session) return;
        const {value, start, end} = snapshot(event);
        if (middle(value) === null || !Number.isInteger(start) || !Number.isInteger(end)) {
            limitation("beforeinput does not expose a correlatable selection", event);
            return;
        }
        const left = session.prefix.length, right = value.length - session.suffix.length;
        if (end < left || start > right) {
            limitation("selection moved outside the active replacement range", event);
            return;
        }
        session.prefix = value.slice(0, Math.min(start,left));
        session.suffix = value.slice(Math.max(end,right));
    }

    /** Sends composing drafts without authorizing aspirations or guessing finality. */
    function input(event) {
        if (session) {
            if (session.ended && !event.isComposing && !terminal(event)) {
                emit("cancel", event);
                session = fence = null;
                emit("input", event);
            } else if (middle(element.value) === null) {
                limitation("input changed text outside the captured replacement range", event);
            } else if (!complete(event)) {
                emit("draft", event);
            }
            return;
        }
        if (terminal(event) || event.isComposing) {
            if (fence && fence.value === element.value && fence.data === event.data) {
                fence = null;
                restore();
                return;
            }
            // A retired session's delayed terminal mutation never becomes an ordinary edit.
            restore();
            return;
        }
        fence = null;
        emit("input", event);
    }

    /** Ends composition using the captured full value or waits for a matching terminal input. */
    function end(event) {
        if (!session) return;
        if (typeof event.data !== "string") {
            limitation("compositionend did not supply its final passage", event);
            return;
        }
        session.ended = true;
        session.final = event.data;
        complete(event);
    }

    /** Installs a local listener whose exceptions become explicit source diagnostics. */
    function listen(type, handler) {
        const listener = event => {
            if (stopped || !element.isConnected) return;
            if (element.disabled || element.readOnly) {
                session = fence = null;
                if (event.type === "input") restore();
                return;
            }
            try { handler(event); }
            catch (error) {
                try { emit("error", event, {message:String(error)}); } catch { /* Eval completion reports a broken sender. */ }
                finally { close(); }
            }
        };
        listeners.push([type, listener]);
        element.addEventListener(type, listener, true);
    }

    /** Applies host resets and sequence-guarded display without an async native-value read. */
    function receive(message) {
        if (stopped) return;
        if (message.kind === "close") { close(); return; }
        if (message.kind === "reset") {
            epoch = message.epoch;
            session = fence = null;
            element.disabled = message.disabled;
            element.readOnly = message.readonly;
            element.value = message.value;
            lastValue = message.value;
            lastSelection = [message.value.length,message.value.length,"none"];
            emit("ready");
        } else if (message.kind === "display") {
            if (message.epoch === epoch && message.sequence === String(sequence) && !session
                && lastValue !== message.value) {
                element.value = message.value;
                lastValue = message.value;
                lastSelection = [message.value.length,message.value.length,"none"];
            }
        } else {
            throw new Error("Unknown numeric source command");
        }
    }

    try {
        listen("compositionstart", start);
        listen("compositionupdate", () => {});
        listen("select", () => {
            if (element.value === lastValue) {
                lastSelection = [element.selectionStart,element.selectionEnd,element.selectionDirection];
            }
        });
        listen("beforeinput", before);
        listen("input", input);
        listen("compositionend", end);
        listen("focus", event => emit("focus",event));
        listen("blur", event => { session = fence = null; emit("blur",event); });
        listen("keydown", event => {
            if (session?.ended && !event.isComposing) {
                session = fence = null;
                emit("cancel", event);
            }
            if (session || event.isComposing) return;
            if (event.key === "ArrowUp" || event.key === "ArrowDown") {
                event.preventDefault();
                emit(event.key === "ArrowUp" ? "up" : "down",event);
            }
        });
        emit("ready");
    } catch (error) { close(); throw error; }
    return {receive,close,closed};
}

/**
 * Owns one bidirectional eval channel on desktop and web until explicit closure.
 * Source identity contains decimal digits only; setup never accesses unrelated inputs.
 */
export async function runNumericEvents(root, token, transport) {
    let source;
    try {
        if (!/^[0-9]+$/.test(token)) throw new Error("Invalid numeric source identity");
        const element = root.document.querySelector(`[data-adam-number-source="${token}"]`);
        if (!element) throw new Error("Numeric source element is not mounted");
        source = createNumericSource(element, token, frame => transport.send(frame));
        const receiving = (async () => {
            for (;;) {
                const message = await transport.recv();
                source.receive(message);
                if (message.kind === "close") return;
            }
        })();
        await Promise.race([receiving, source.closed]);
    } catch (error) {
        transport.send({token,action:"fatal",message:String(error),epoch:"0",sequence:"0",session:"0",value:""});
    } finally { source?.close(); }
    return null;
}
