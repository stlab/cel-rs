# Host-visible propagation outcomes

**Status:** Approved design for issue #273.

## Intent

This design defines how a Rust host handles successful propagation outcomes and
recovers from errors. A failed mutating operation leaves the `Sheet` state
unspecified, so the host discards that sheet and rebuilds it from host-owned
inputs. The host does not need to know whether propagation published derived
values before returning an error.

## Audience and scope

This document is for developers embedding `adam-rs` and for the implementer of
issue #273. It covers incremental graph construction, explicit writes and
batches, propagation errors, validation diagnostics, and recovery. Issue #272
will consume this contract in its framework adapter and recording example.

The design does not add transactions across writes, roll back host-visible
effects, or choose a recording format. A host reports invalid model state with a
false requirement; that diagnostic does not roll back values. A callback `Err`
represents a runtime failure, not a validation rejection.

## Current behavior

`Sheet` is assembled incrementally through mutating declaration methods.
`Sheet::write` immediately replaces a cell's source value and clears its derived
override. Each successful write takes effect independently.

`Sheet::propagate` stages planning and method execution before publishing derived
values. It evaluates requirements after that publication. Therefore the same
`Error::MethodFailed` can arise before or after derived values are published.
Under the project error contract, the host discards the mutated sheet after
either error, so the commit phase does not change the host's recovery action.

A requirement that returns `false` is not an error. Propagation completes and the
sheet reports the violated requirement. Filter callback failures are also
nonfatal diagnostics. These outcomes differ from a callback that returns `Err`.

The relevant behavior is documented by `Sheet::write` and `Sheet::propagate` in
`adam-rs/src/sheet.rs`. Existing contract regressions in
`adam-rs/tests/contract_regressions.rs` cover staged and post-publication
`MethodFailed` cases.

## Goals

- Keep the existing fallible API and treat every mutation error alike: the host
  discards the sheet and rebuilds it from host-owned inputs.
- Do not add a typed error or commit-phase flag. The host takes the same recovery
  action whether propagation failed before or after publishing derived values.
- Preserve the distinction between an ordinary validation result and a runtime
  failure. A false requirement and a filter failure remain nonfatal diagnostics.
- State that explicit writes take effect immediately and a sequence of writes is
  not atomic. A host that needs to reject a batch keeps its last accepted state
  outside the sheet and rebuilds from that state after an error.
- Keep snapshot capture and replay based on successful operations and the
  existing effective-value contract. A host must not capture from a sheet after
  a mutating operation returns `Err`.

## Proposed contract

### Writes

`Sheet::new` creates an empty sheet. Graph declaration, validation, writes, and
propagation then mutate that sheet. When a mutating operation returns `Err`, its
state is unspecified under the project error contract. Do not inspect, retry, or
capture from that sheet; discard it and rebuild from host-owned inputs. This
recovery rule applies during graph construction as well as during edits.

Each successful `write` immediately updates its source value. A sequence of
writes is not atomic. If an operation in a batch returns `Err`, discard the
sheet and reconstruct it from the host's last accepted inputs rather than
trying to undo or inspect individual writes. Errors from read-only queries are
outside this mutation recovery rule.

### Propagation

Keep `propagate` fallible with its existing `Error` result. The host handles
every propagation error by discarding the sheet; it does not inspect the error
to determine whether derived values were published. The result drives host
behavior as follows:

| Result | Meaning | Caller action |
| --- | --- | --- |
| `Ok(())` with all requirements satisfied | Derived values and diagnostics describe the completed propagation. | The host may publish the values and apply its recording policy. |
| `Ok(())` with a false requirement or filter diagnostic | Propagation completed; the sheet reports a validation or filter diagnostic. A false requirement does not roll back values. | The host handles the diagnostic as validation state, not as a runtime error. |
| `Err` from `propagate` or another mutating operation | The sheet's state is unspecified; the error may occur before or after partial changes. | Do not inspect or capture from the sheet. Discard it and rebuild from host-owned state. |

Every `Err` from `propagate` requires the same recovery, including planning,
dependency, method, type, and requirement-callback errors. The error does not
promise whether derived values were published. The host owns the authoritative
inputs and reconstructs the model from them.

False requirements are the supported way to report a rejected or invalid
application state during propagation. They complete propagation and remain
inspectable through the requirement diagnostics, but do not roll back values.
Filter failures remain nonfatal diagnostics and do not turn propagation into
`Err`. An error returned while adding a requirement follows the same mutating
operation recovery rule.

### Callbacks

Method, filter, conditional-expression, and requirement callbacks remain
side-effect-free. The API does not guarantee whether, when, or how often a
callback runs. A callback error is not an application-level rejection and does
not permit reuse of the failed sheet.

### Capture and replay

The host may use `Sheet::source_cells` and effective `read` values after
successful propagation. It must not capture or replay from a sheet after a
mutating operation returns `Err`.
The host chooses whether a successfully propagated state with false requirements
is recordable; the core library does not make that application policy decision.
Issue #272 demonstrates one explicit policy and keeps UI state, model state, and
recordings consistent.

## Non-goals

- Roll back explicit writes or make a sequence of writes atomic.
- Guarantee that a failed `Sheet` remains readable or reusable.
- Add a stronger failure guarantee than the project-wide error contract.
- Distinguish propagation failure phases when the host discards the sheet for
  every propagation error.
- Convert callback errors into validation results.
- Define host UI, persistence, or recording-format policy.
- Guarantee callback order, count, or execution.

## Acceptance criteria

1. The existing fallible API remains sufficient; no propagation phase
    classification is added because the host discards the sheet after any
    propagation error.
2. The recovery contract directs the host to discard and reconstruct after an
    error from a mutating operation, including during graph construction, writes,
    validation, or propagation.
3. The public contract states that writes are immediate and batches are not
    atomic. The host keeps its last accepted inputs outside the sheet and does
    not inspect or capture from the sheet after a mutating error.
4. A false requirement returns a completed propagation result with validation
   diagnostics, not a runtime error. A filter failure remains a nonfatal
   diagnostic.
5. Public-contract tests cover representative mutation errors, false
    requirements, filter failures, and recovery by constructing a fresh sheet
    from host-owned inputs. Tests do not inspect or reuse a sheet after a
    mutating method returns `Err`.
6. The #272 framework adapter handles every mutation error by keeping its
    published UI and recording state aligned with host-owned state, and
    demonstrates validation rejection with a false requirement rather than a
    callback error.
7. Documentation explains the capture boundary and the host's responsibility to
    rebuild after a mutating-operation error.

## Implementation choices

Keep the existing `Error` API. Do not expose internal planning stages or commit
phase information that callers do not need to implement this contract.

Keep the #273 runtime API and contract tests in the `adam-rs` worktree. Issue
#272 owns the runnable host adapter and recording example and should consume the
merged #273 API in its own worktree.
