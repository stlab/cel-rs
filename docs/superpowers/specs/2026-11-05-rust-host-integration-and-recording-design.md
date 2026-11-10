# Dioxus sheet bindings and recording

**Status:** Approved on 2026-11-05 for issue #272, including the Result-only
construction contract.

## Intent

Give Rust developers a Dioxus integration that binds a widget to a named Adam
cell without repeating read, write, propagation, or refresh plumbing. A runnable
Dioxus example demonstrates the binding and records and replays accepted
source-state changes. `adam-rs` and `adam-lang` remain independent of Dioxus.

## Audience and scope

This design is for developers embedding Adam in Dioxus applications and for the
implementer of issue #272. It covers reusable Dioxus controls, their Adam binding
layer, a runnable example, task-oriented integration guidance, and migration of
the existing component consumers.

Replace all existing Spectrum widgets in `begin` with Dioxus components. Migrate
`ez-adam` and the book/slides live embeds to the same component family rather
than preserving the Spectrum wrappers. `ez-adam` remains a document editor; this
migration does not add runtime Sheet evaluation or redesign its persistence.
If a separate runtime error-recovery change becomes necessary there, open a
separate issue before making that change.

This first pass supports the widgets these applications already use, not every
Dioxus component type. Additional bindings, including date inputs, are tracked
in [issue #279](https://github.com/stlab/cel-rs/issues/279).

## Existing defects and migration dependencies

The existing `adam-web-ui::build_sheet` can return a failed `ParsedSheet`
alongside an error through `BuildOutcome`. This is a defect to remove, not
the construction interface to preserve. Replace `BuildOutcome` with a
`Result` whose error variant contains no sheet. The inspector's
`write_and_propagate` also reports write and propagation failures internally,
then keeps the same parsed-sheet signal. `ViewConstraints` owns a persistent
`ParsedSheet`; its update methods return an error without invalidating the
containing value.

`begin::load_example` reports build errors and installs any returned sheet,
including one whose initial propagation failed. These paths conflict with the
recovery contract in
[`host-visible propagation outcomes`](2026-11-04-host-visible-propagation-outcomes-design.md):
after a mutating operation returns `Err`, the sheet is unspecified and
discardable.

`ez-adam` edits its own `Document`, persists that document as JSON, and uses
`adam-web-ui` for Spectrum components. It does not evaluate or own an
`adam-rs::Sheet`. Its side panel and toolbar import the shared Spectrum wrappers,
and its application loads a separate Spectrum bundle.

The book's live root also wraps the shared inspector in a Spectrum theme.
The book/slides asset staging copies `begin`'s Spectrum bundle and inspector
styles, and the browser bootstrap loads that bundle. The migration must update
these consumers together; removing only `begin`'s widgets leaves the shared
inspector and its assets inconsistent.

The [Dioxus component project](https://github.com/DioxusLabs/dioxus-components)
provides customizable styled components over unstyled primitives. Its `Input`
forwards native input, focus, keyboard, clipboard, and composition events.
It does not supply the numeric draft, conversion, or Sheet binding contract
described here. Those behaviors belong in the reusable editor and binding
layers, not in application-specific DOM scripts.

## Goals

- Give a library consumer a runnable Dioxus example and a guide that starts with
  dependency setup, installs application types and operations, constructs a
  model, propagates initial values, and binds widgets to named cells.
- Put Dioxus integration in `adam-web-ui`, without adding a Dioxus dependency
  to `adam-rs` or `adam-lang`.
- Express a widget binding through a cell handle that carries reactive value
  and enabled, read-only, relevant, and invalid state.
- Supply reusable Dioxus controls, including a locale-aware numeric editor,
  without requiring those base controls to own a runtime Sheet.
- Replace Spectrum controls in `begin`, `ez-adam`, and the book/slides embeds
  while preserving their existing application behavior.
- Make source construction, input validation, false requirements, and runtime
  propagation failures visible to the host and to the developer.
- Keep the last accepted application transaction and recording state outside
  the mutable `Sheet`.
- Exercise capture and replay through the public source-state API added by
  issue #271.
- State ownership, thread-affinity, and feedback-loop boundaries without
  promising `Send` or `Sync`.

## Non-goals

- Change the `Sheet` mutation-error contract established by issue #273.
- Add transactions, rollback, persistence formats, or UI dependencies to
  `adam-rs` or `adam-lang`.
- Add a generic binding DSL, live RSX reconfiguration, C/C++ bindings, or the
  full development environment described by issue #113.
- Add runtime Sheet evaluation, persistence redesigns, or unrelated
  error-recovery changes to `ez-adam`.
- Bind every Dioxus component type in this pass.
- Record playback-generated updates as new user actions.

## Design

### Dioxus integration and binding interface

The target consumer expression is:

```rust
rsx! {
    EditNumber {
        bind: sheet.cell("cell_name")
    }
}
```

This is the proposed binding shape, not an existing API or a complete runnable
example. Here, `sheet` is a Dioxus integration handle, not a local mutable
`adam_rs::Sheet`. The integration constructs a named sheet from an `.adm2`
module and retains one model owner across component renders. The guide shows
the complete setup, including application operation/type registration, model
selection, construction errors, and the application outcome handler.

A cell binding connects the control's displayed value and state to that owner.
It submits edits through the owner's transaction path and refreshes dependent
controls after propagation. Application code does not repeat those steps in
each widget's event handler. Looking up an unknown name or binding an
incompatible type reports a structured configuration diagnostic; it must not
panic, install an empty sheet, or silently create an inert binding.

The owner serializes transactions and publishes coherent value/state updates.
Derived cells remain editable when the model permits a write; being derived
does not itself mean disabled or read-only. Forced values and cells disconnected
from the relevant outputs disable their input controls. Explicit output cells
remain non-editable, with diagnostics visible and their values available for
inspection. The binding exposes these states rather than inferring them from
source classification alone.

The integration retires bindings when their model instance is replaced or
discarded. An event or pending response from an old instance cannot mutate the
replacement, even when both contain a cell with the same name. The owner
forwards outcomes to the application boundary; a widget does not choose
recovery or shutdown policy.

### Reusable controls and component migration

Use the official Dioxus components and primitives where they fit. Add custom
controls where the existing components do not implement the required behavior.
Keep base controls usable with controlled values and callbacks independently
of Adam. The binding-facing controls adapt those callbacks and state to a
Sheet owner while retaining the concise `bind` interface.

Replace the existing numeric and text fields, checkbox, slider with numeric
editing, switch, action buttons/groups, navigation items, labels, help text,
headings, separators, icons, and theme wrapper where those appear in the
applications. Use ordinary Dioxus elements for structural roles that do not
need a dedicated component. Preserve keyboard access, accessible names,
selection state, validation messages, file actions, and graph zoom controls.
This is a component migration, not an application layout redesign.

`ez-adam` uses the base controls with its existing `Document` operations; it
does not acquire a runtime Sheet to use the components. The shared inspector,
the book live root, and the slides use the same component styling and binding
behavior as `begin`. The book/slides retain independent model owners per live
example and their existing graph associations.

Update theme tokens, inspector styles, asset staging, browser bootstrap, and
affected tests together. Remove the obsolete Spectrum wrappers, custom-element
value readers, initialization, and bundles after migrating their consumers.
Retain unrelated JavaScript, including graph rendering and file integration.
Do not regenerate a Spectrum bundle merely to remove its consumers.

### Numeric editing contract

`EditNumber` supports locale-aware display and entry, including grouping and
decimal separators. Parsing and formatting use the same resolved locale.
Conversions preserve the bound numeric type and its range; integer values must
not round-trip through an imprecise JavaScript `Number` or `f64`.

The editor distinguishes its local text draft from the sheet's effective value.
Its observable behavior is:

| Event or state | Behavior |
| --- | --- |
| Input forms a complete number representable by the bound type | Submit the value immediately through the transaction path; no Enter key or explicit commit is required. |
| Input is an incomplete numeric draft, such as a sign or decimal separator | Keep the draft locally without sending a value or propagating. |
| Input cannot convert to the bound numeric type | Keep the draft and show the input error without mutating the sheet. |
| The sheet changes while the focused editor contains an incomplete draft | Preserve the draft and retain the latest sheet value separately. |
| The draft becomes a valid complete number | Submit it immediately and resume synchronization with the resulting sheet state. |
| Focus leaves an editor with an incomplete or invalid draft | Replace the draft with the latest sheet value, or the last valid displayed value when no newer value exists. Do not resend that value. |
| A sheet update forces or disconnects the cell | Discard the draft immediately, display the sheet value, and disable the editor. |
| The model is replaced or discarded | Retire the draft and its binding; do not send a delayed edit to another model. |

Focus loss and programmatic synchronization never count as user edits. A valid
number submitted by the editor still passes through model validation; lexical
validity does not guarantee that the host accepts the transaction.

Preserve the editing position and equivalent entered formatting while typing
rather than reformatting the field on every render. Keyboard stepping and
increment/decrement controls respect numeric type, step, range, and disabled
state. The model's filter remains authoritative for effective values and
aspirations; the widget must not silently clamp an edit before submitting it.
A range change must not display a different numeric value from the model
merely because a slider's bounds changed.

The user-authorized native event adapter captures full values, UTF-16 selection,
input type, and composition state synchronously at the element. The same
component-owned eval channel transports these snapshots on web and desktop.
Composition completion validates an actual full snapshot against the
source-captured replacement span and final passage; it never submits a
reconstructed value or infers finality from elapsed time. Both pre-end-only
and delayed post-end native input orders remain supported. Terminal composition
echoes do not count as another edit; ordinary equal-valued edits still do.
Source sessions retire on blur and replacement; acknowledged source epochs
reject queued events from before a forced reset.
Missing authoritative selection or an uncorrelatable range expansion produces
an explicit `InputError::EventSource` limitation, not a guessed write. Neither
this adapter nor formatting asynchronously reads the native input value.

### Host ownership and error boundary

Model construction returns a `Result`. Only `Ok` contains the constructed
sheet and its associated successful-build metadata, such as labels. Parsing,
type checking, initial propagation, and initial requirement validation must
succeed before construction returns `Ok`. An `Err` contains a structured error
and any independently owned diagnostic metadata needed by the host; it never
contains a sheet, whether as a partial result or an optional recovery payload.
Initial propagation errors retain their fatal classification rather than
becoming ordinary source-validation errors.

On failure, the builder discards its candidate without reading it again.
Diagnostics use the returned error and metadata retained before the failing
mutation. Update all construction call sites to handle the `Result`; do not
preserve a wrapper that can return a sheet and an error together.

The host owns the authoritative inputs, the last accepted transaction, the
active model, and any recording state. A UI component or adapter may apply an
input to a `Sheet`, but it does not decide whether the application rolls back,
keeps running, persists state, or exits. It returns structured outcomes to its
caller instead of swallowing an error, formatting away its type, or silently
substituting an empty sheet.

Any `Sheet` whose mutating operation returns `Err` is unspecified but safely
discardable. The component must not read, inspect, capture, retry, return as
active, or otherwise reuse that sheet. Error diagnostics must use the returned
error and metadata retained independently of the failed sheet, such as source
text, spans, and cell labels.

The host distinguishes rejected input from a runtime propagation failure:

| Outcome | Host action |
| --- | --- |
| Source parsing, type checking, or input conversion rejects a candidate | Report the diagnostic and keep the last accepted application transaction. If no transaction has been accepted yet, show the error without installing a model. |
| Propagation succeeds but a requirement returns `false` | Report the invalid requirement. Do not commit the candidate transaction; restore the last accepted state from host-owned inputs. A false requirement is a validation result, not a `Sheet::propagate` error. |
| A filter reports a nonfatal diagnostic | Show the diagnostic without treating it as a runtime propagation error. The example states its application policy for the resulting value. |
| `Sheet::propagate` returns `Err` | Report the runtime failure, discard the failed sheet, and do not rebuild or retry around the failure. Before aborting, attempt to persist the last accepted transaction if it is not already durable. The top-level application chooses how it terminates; process termination is the recommended example policy. |
| A recoverable mutation rejects a transaction | Report the error and reconstruct from the last accepted host-owned inputs. Never inspect or reuse the failed sheet. |

The component forwards all errors and validation outcomes to the client. This
includes errors raised during initial model construction and errors raised by
later UI events. A host that reloads source keeps its prior accepted model while
it evaluates the candidate; it publishes the candidate only after construction,
initial propagation, and requirement validation succeed.

The adapter contract specifies the observable outcomes, not a particular Rust
enum or callback signature. The implementation may choose the simplest typed
interface that preserves the distinction between rejected input, validation
state, and a fatal propagation error. It must not reduce these outcomes to an
ambiguous string-only success/failure channel.

### Transaction and recording lifecycle

The host applies a user action to a candidate transaction while retaining the
last accepted inputs separately from the active sheet. It commits the action
and records it only after propagation succeeds and the application's
requirements pass. A rejected candidate restores the accepted transaction
from those saved inputs. A propagation `Err` follows the fatal path above and
does not become a replayable action.

Recovery state and action recordings serve different purposes. The host retains
accepted input aspirations and the input ordering needed to restore relationship
direction, as well as compatible model source and configuration. It must not
replace those aspirations with filtered effective values. An ordered journal
of accepted typed input operations is an acceptable host-side representation;
the builder may choose another representation that preserves the same behavior.
Rejected operations never enter that recovery state. A restoration failure is
reported through the same error boundary rather than hidden by an empty model.

Capture uses `Sheet::source_cells` and effective `Sheet::read` values after
successful propagation. It does not capture source aspirations, transient
`CellId` values, or a failed sheet. Replay targets a fresh compatible model,
writes all captured values before propagation, and suppresses recording while
playback runs. The example uses stable host-owned names or schema keys for any
persistent representation. These recordings reproduce the captured effective
source state; they are not a substitute for the host's aspiration-preserving
recovery state.

The demonstrated model contains a multi-way relationship, a conditional, a
filter that retains a user's aspiration, and a requirement. The UI and guide
distinguish whether a cell is currently a source from whether it is forced or
read-only; a writable derived cell can change relationship direction.

### Framework and execution boundaries

The guide states the framework-neutral event-loop contract: marshal input
events to the single owner of the model, apply one transaction at a time, and
publish refreshed values and cell state only after the outcome is known.
The runnable example implements that contract in Dioxus rather than introducing
a second, framework-neutral example. Callbacks do not re-enter model mutation
while propagation is in progress. Playback-generated updates do not feed back
into the recorder.

The standalone example and `begin` use the same Dioxus binding. Their outermost
application boundary receives build, validation, and runtime outcomes from
`adam-web-ui`; it owns the accepted state and chooses reporting, rollback, and
shutdown behavior. `adam-web-ui` supplies the binding and UI-facing metadata,
but does not retain or recover a failed `Sheet`. The book/slides live roots
likewise handle outcomes at their embed boundary, without inspecting or
continuing to drive a failed model.

No component promises that a `Sheet`, adapter, or callback is `Send` or `Sync`.
The example confines each live sheet to its owning UI/event-loop thread. A host
that dispatches work across threads must marshal results back to that owner
before accessing or replacing the model.

### Documentation and runnable example

The standalone Dioxus example and task-oriented guide show the complete
consumer lifecycle: add the library and Dioxus integration, register types and
operations, select and instantiate a named sheet, propagate the model, bind a
widget, report a rejected requirement, capture an accepted action, and replay
it against a fresh compatible model. They include exact run commands and
expected observable values. The example uses explicit bindings rather than
only rendering the generated inspector.

The guide links from the repository README and the Adam book's embedding
reference. It explains the host's ownership and failure responsibilities,
identifies the adapter boundary, and does not present `begin` or `ez-adam` as a
required host framework.

## Verification

- Test the Dioxus binding's input, output, commit, rejection, and update
  decisions using public APIs.
- Test that a widget edit refreshes other bound widgets and supports
  relationship direction changes without application-written event plumbing.
- Test numeric draft handling, focus-loss restoration without a write,
  external updates during incomplete edits, and immediate draft retirement
  when the cell becomes forced or disconnected.
- Test locale-aware grouping and decimal entry/display under multiple locales,
  exact large integer conversion, type-range errors, stepping, and filter
  corrections without pre-clamping away aspirations.
- Test unknown names, incompatible widget/cell types, source replacement, and
  stale events without a panic or a write to the wrong model.
- Test that a recorded accepted action replays against a fresh compatible model
  and that playback does not create another recording.
- Test that rejecting a later transaction restores earlier accepted aspirations
  and relationship direction, including when a filter's effective value differs
  from the accepted input.
- Test that a false requirement is reported as validation state and that a
  propagation `Err` is surfaced as fatal without reading or reusing its sheet.
- Test construction and input-conversion errors without replacing the last
  accepted model.
- Test that construction returns `Ok` only for an initialized, validated model
  and that every construction failure returns an error without a sheet.
- Render-verify the standalone example and migrated `begin` widgets, including
  visible validation and runtime-error outcomes, keyboard/focus behavior, and
  live dependent updates.
- Render-verify `ez-adam`'s migrated controls without changing its document
  operations, persistence, or runtime model.
- Verify book/slides live inspectors, with and without graphs, using the
  migrated assets and no Spectrum dependency.
- Verify the documented run commands and expected values.

## Open implementation choices

- Choose the standalone Dioxus example location and the model-owner setup API.
  Preserve the target `EditNumber { bind: sheet.cell("cell_name") }` shape,
  allowing explicit setup-time error handling and type information where needed.
- Choose which official components/primitives to reuse, the smallest base-control
  interface, and the locale parsing/formatting mechanism. Document supported
  numeric types and locale behavior, and preserve the contracts above.
- Choose how `begin` displays recoverable diagnostics and how it attempts to
  persist the last accepted transaction before fatal shutdown.
- Keep `ez-adam` changes limited to the agreed component migration and directly
  dependent code; open a separate issue for unrelated runtime recovery work.

The distinction between reusable base controls and Adam binding controls, and
preserving equivalent focused text to avoid caret jumps, are design choices
proposed by the assistant for approval with this document.
