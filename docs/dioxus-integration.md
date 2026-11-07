# Bind an Adam sheet to a Dioxus application

This guide helps Rust application developers run a standalone Dioxus consumer,
bind named cells, and choose recovery and recording policy. It covers the host
lifecycle, not the Adam language syntax or a general application framework.

## Run the example

You need Rust and your platform's Dioxus desktop prerequisites. On macOS, install
Apple's command-line developer tools. On Windows, install the Visual C++ build
tools and WebView2 runtime. On Linux, install the WebKitGTK dependencies required
by Dioxus desktop. The example uses Dioxus 0.7.10 and the library's pinned official
Dioxus controls and `Theme`; it needs neither Node.js nor a JavaScript bundle.

The library compiles its official theme and control stylesheets with Lightning
CSS, then installs the self-contained stylesheet through Dioxus's document API.
Plain Cargo therefore shows the official control styles without a JavaScript
bundle or the Dioxus asset bundler. You can exercise the range filter through the
slider's numeric entry.

1. From the repository root, launch the desktop example:

   ```bash
   cargo run -p adam-web-ui --example sheet-binding --features desktop
   ```

2. Confirm **Width** is `4`, **Height** is `3`, and **Area** is `12`.
3. Enter `24` in **Area**. **Width** becomes `8`; **Height** stays `3`.
4. Enter `24,5` in **Area**. That control explicitly uses `de-DE`, so the comma
   is a decimal separator. **Width** becomes approximately `8.166666666666666`.
5. Enter `200` in **Area**. The named `area_limit` requirement rejects the
   candidate before publication. The status shows the source diagnostic, and the
   host explicitly restores the last accepted inputs.

The [complete example](../adam-web-ui/examples/sheet_binding.rs) contains all
imports, registration, host policy, controls, and contract-derived tests. Its
[module fixture](../adam-web-ui/examples/sheet_binding.adm2) contains two roots;
the application selects `consumer::demo`, not `unselected`.

## Configure a consumer

For an application next to a `cel-rs` checkout, add these dependencies to your
application's `Cargo.toml`:

```toml
[dependencies]
adam-web-ui = { path = "../cel-rs/adam-web-ui", features = ["desktop"] }
adam-lang = { path = "../cel-rs/adam-lang" }
cel-parser = { path = "../cel-rs/cel-parser" }
cel-runtime = { path = "../cel-rs/cel-runtime" }
dioxus = { version = "=0.7.10", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"

[dev-dependencies]
dioxus-ssr = "0.7.10"
```

Adjust the paths to your checkout. `adam-web-ui/desktop` forwards
`dioxus/desktop`. Without that feature, the library selects no renderer.
A web application selects `dioxus/web` itself and enables `adam-web-ui/web`.
The standalone example target requires `desktop`.

Copy `sheet_binding.rs` to your application's `src/main.rs` and
`sheet_binding.adm2` next to it to use the complete application. Its imports are:

```rust
use adam_lang::{ModuleCompiler, ModulePath, ModuleSet, TypeRegistry};
use adam_web_ui::{
    BuildError, Renderer, ScalarCodec,
    binding::{
        CellEdit, CheckBox, EditNumber, EditSlider, EditText, HostOutcome,
        HostRejection, ModelFactory, PlaybackError, Recording, RecoveryState,
        SheetEvent, SheetHandle, SheetOwner, UpdateOrigin, use_sheet,
    },
    components::Theme,
    initialize_sheet,
};
use cel_parser::{OpLookup, OperationSignature, ResolvedLeafType};
use cel_runtime::ValueType;
use dioxus::prelude::*;
use serde::Serialize;
use std::{io::Write, path::Path, rc::Rc};
```

## Register types and select a root

The example's `Badge(u32)` is an application scalar with default `7`. It has
three separate registrations:

1. `types.register::<Badge>("Badge")` makes the Rust type available to Adam.
2. `lookup.register_operation(OperationSignature { ... })` registers the typed
   `badge.badge_number()` operation. Its emitter uses
   `segment.op1(|badge: Badge| badge.0)` with a `Badge` operand and `u32` result.
   Operations that can fail must use an `opNr` emitter.
3. `built.labels.add_cell_with_codec(...)` supplies an exact UI codec.
   The codec encodes `badge.0.to_string()` and decodes `text.parse::<u32>()`,
   returning `InputError::Lexical` on malformed or overflowing input.
   Type registration alone does not provide a UI codec.

`demo_factory` compiles a `ModuleSet` with the application's `TypeRegistry` and
`OpLookup` once. The retained compiled modules own their immutable source and
operation metadata. Each factory invocation instantiates the explicit selection,
initializes it, and attaches the application codec:

```rust
let parsed = compiled
    .instantiate_root(&module, "demo")
    .map_err(BuildError::Module)?;
let mut built = initialize_sheet(parsed)?;
built.labels.add_cell_with_codec(
    built.keys["badge"],
    "badge",
    ScalarCodec {
        encode: encode_badge,
        decode: decode_badge,
    },
);
Ok(built)
```

Here `module` is the parsed `ModulePath` `"consumer"`; `compiled` comes from
`ModuleCompiler::new(types, lookup).compile(modules)`. The example reports every
compilation diagnostic with `format_module_diagnostic`. It never chooses the
first root implicitly or installs an empty model on failure.

The factory reproduces the same graph, initial values, keys, and codecs on each
invocation. Change the schema key when any of those contracts changes.

## Bind controls and handle outcomes

`App` retains the compiled factory with `use_hook`. `use_sheet` initializes one
owner per mounted hook and caches its success or construction failure across
renders. Changing factory props does not replace that owner; replacement is an
explicit operation. The application passes its handler to the hook:

```rust
use_sheet(
    factory.clone(),
    SCHEMA.into(),
    EventHandler::new(move |event| handle_event(event, status)),
)
```

Inside `Theme`, controls address symbolic names, not runtime cell IDs:

```rust
rsx! {
    Theme {
        EditNumber { bind: sheet.cell("width"), label: "Width" }
        EditNumber { bind: sheet.cell("area"), locale: "de-DE".to_owned() }
        EditText { bind: sheet.cell("badge"), label: "Application badge" }
        CheckBox { bind: sheet.cell("rectangle"), label: "Rectangle" }
        EditSlider { bind: sheet.cell("limited"), label: "Limited aspiration" }
    }
}
```

The control submits complete aspirations through one transaction path. An
incomplete numeric draft such as `-` remains local and does not enter accepted
history. Blur restores the accepted display without writing. Native composition
uses the control's event-source adapter, not asynchronous DOM-value reads.
Unsupported or unobservable native selection transitions report `EventSource`
errors rather than guessing an edit; [issue #280](https://github.com/stlab/cel-rs/issues/280)
tracks that boundary.

*Source classification* describes the last accepted solver direction, not
writability. **Area** initially derives from **Width** and **Height**, but remains
editable because the relationship can reverse direction. Forced or disconnected
inputs disable their controls; explicit `out` cells are read-only.
`CellBinding::view` exposes these flags and diagnostics. Unknown names,
incompatible controls, and missing exact codecs report configuration errors.

The example's host policy distinguishes these outcomes:

| Outcome | Application action |
| --- | --- |
| `Accepted` | Keep the coherent publication and accepted history. |
| `RecordingFailed` | Report the stopped recorder. Keep the newly accepted model, publication, and history. |
| `Rejected(Input)` or `Rejected(Binding)` | Report the input/configuration problem without automatic reconstruction. |
| `Rejected(Validation)` or `Rejected(Mutation)` | Report owned source diagnostics, then explicitly call `restore_accepted`. Report restoration errors too. |
| `Fatal` | Report the owned runtime diagnostic, attempt accepted recovery persistence, then exit nonzero. Never restore, retry, or replace the retired model. |

A rejected owner transaction preserves accepted views and history but makes live
inspection unavailable until explicit restoration. After an input error, use
**Restore accepted inputs** before further model edits. A draft error reported
by a control before a transaction need not discard the model.

## Verify conditional and filter behavior

The fixture fixes **Height** as a `source`. With **Rectangle** selected, its
relationship computes a product; with it cleared, the relationship computes a
sum. Starting from a fresh model, clear **Rectangle** and enter `24` in **Area**:
**Width** becomes `21`, with **Height** still `3`.

The filter starts with **Limited aspiration** `5` and **Ceiling** `10`.
Enter `20` in **Limited aspiration**: the effective value is `10`, while accepted
history retains `20`. Raise **Ceiling** to `30`: the effective value becomes
`20`, demonstrating the retained aspiration. The `positive_width` requirement
also rejects nonpositive width; `area_limit` requires **Area** at most `100`.
Requirements are language diagnostics; this binding's host transaction policy
uses false requirements to reject publication.

## Capture and replay effective sources

1. Restart the example to get `width = 4`, `height = 3`, `area = 12`.
2. Select **Start recording**, then enter `24` in **Area** and `20` in
   **Limited aspiration**.
3. Select **Capture**. `sheet.capture(&[])` captures all contributing effective
   sources. You can instead select result keys such as `&["area"]`.
4. Change **Area** to `30`, then select **Fresh playback**. The host calls
   `sheet.playback(factory, SCHEMA.into(), &recording)` on a fresh factory
   instance. **Area** returns to `24`, **Width** to `8`, and **Limited aspiration**
   to `10`. The fresh owner has zero user recordings.

`Recording` serializes with `serde_json::to_string(&recording)` and deserializes
with `serde_json::from_str::<Recording>(&json)`. It stores schema identity and
exact effective source encodings by stable keys. Playback applies the complete
group before one propagation, validates compatibility, and suppresses recording
feedback. It does not restore filtered aspirations: the captured limited value
is `10`, not the original `20`.

## Persist fatal recovery inputs

Expand **Deliberate fatal runtime test** and select **Inject runtime failure**.
The host sets `divisor` to zero, triggering integer division failure. It reports
the diagnostic, attempts to write `sheet-binding-recovery.json` in the launch
directory, reports any persistence error, and exits with status `1`.
Use a separate process to verify termination without closing a test harness:

```bash
cargo run -p adam-web-ui --example sheet-binding --features desktop \
  -- --fatal-check
```

That explicit test mode accepts `area = 24` and the limited aspiration `20`,
then triggers the same fatal propagation and persistence boundary without opening
a window. Expect exit status `1`. The JSON contains the module source, file name,
module/root selection, schema key, and those two accepted ordered transactions.
It excludes the rejected divisor edit.

Recovery data differs from an effective-source recording. The example borrows
the owner's one accepted journal through
`with_accepted_state(|accepted| save_recovery(accepted))`, handling its
`Result` as well as persistence errors. Ordered groups preserve recency and
unclamped aspirations. The document contains no closures or runtime IDs.
The host must supply trusted operation/type/codec registrations when rebuilding
in a later process; JSON does not contain executable application code.
The example writes recovery inputs but does not implement a recovery-file loader.
Remove or archive the named recovery file after examining it.

## Keep ownership on the event loop

Each owner and its `Rc` factory stay on the owning UI/event-loop thread; none
promises `Send` or `Sync`. Marshal worker results back to that owner before model
access. Thread confinement and reentrancy are different constraints: even on one
thread, a second mutation or journal access during a held mutation borrow reports
`ReentrantMutation`.

`inspect` borrows only a live accepted model for a read-only projection.
`with_accepted_state` borrows history even after rejection or fatal retirement.
Do not mutate from either borrow callback. Outcomes raised during a borrow wait
until all enclosing owner borrows end, then reach the host in order. Host outcome
callbacks can read or mutate the handle and explicitly restore recoverable state.

Successful replacement, restoration, and playback retire prior installation
bindings. Reacquire `sheet.cell(...)` during rendering rather than retaining
callbacks from an old installation. If your load path already owns a validated
`SheetOwner`, use `SheetHandle::from_owner` or `install` to transfer it rather
than constructing a duplicate live model.

Run the public-API example tests to verify the fixture and host contracts:

```bash
cargo test -p adam-web-ui --example sheet-binding --features desktop
```
