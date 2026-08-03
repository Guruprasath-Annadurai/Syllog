# Phase 7 implementation ledger

Updated: 2026-08-03

Checked items have focused tests and the full repository gate at their commit.
Unchecked items are not claimed by adjacent infrastructure.

## Gate 7.1 — Async lowering and structured tasks

- [x] `await` syntax with `SYL2501` outside `async fn`
- [x] Explicit verified `Start`, `Suspend`, `Resume`, `Complete`, and `Cancel` transitions
- [x] Conservative live-local frame metadata and one shared drop state
- [x] Deterministic and Tokio schedulers with identical lifecycle order
- [x] Cancellation and panic run the shared drop path exactly once
- [ ] Borrowed-local rejection across suspension
- [ ] Child-task scope exit and general structured task groups

Known limitation: Syllog has no source reference/borrow syntax or borrow checker
yet. Borrowed-local rejection depends on Phase 8 and is not claimed. The current
async MIR is a verified scheduling side table; Wasm code generation does not
yet emit resumable async frames.

## Gate 7.2 — Versioned provider ABI

- [x] Explicit major/minor provider ABI descriptor
- [x] Incompatible ABI and duplicate registration rejection
- [x] Immutable registry snapshots and exact route lookup
- [x] Credential-kind declarations and formatting/JSON-safe secret values
- [x] Explicit idempotent cancellation with cancellation-safe bounded sinks
- [x] Ordered provider terminal failures and retained backpressure behavior
- [x] Adapter-instance credential capability injection

Known limitation: adapter credentials are capability-wrapped and are excluded
from formatting and serialization, but OS keychain/HSM-backed secret providers
do not exist yet. No provider secret is placed in a model request or lifecycle
event.

## Gate 7.3 — Production pipeline executor

- [x] Serial stage execution
- [x] Measured bounded fan-out and structured child joins
- [x] Declaration-order and completion-order join policies
- [x] Paused-time retry backoff and total stage deadlines
- [x] Shared threshold/reset circuit breaker
- [x] Cooperative cancellation waits for child cleanup
- [x] Successful lifecycle logs exclude pipeline payloads
- [ ] Failure results retain their accumulated lifecycle log
- [ ] Forced cancellation grace period and explicit detached supervisor

Known limitation: stage cancellation is cooperative through `StageContext`; a
stage that ignores cancellation must have a deadline to guarantee termination.
Successful outcomes retain ordered lifecycle events, but current error returns
contain only the normalized error, so failure-event retention is not claimed.

## Gate 7.4 — Provider adapters

- [x] Separate `OpenAI`, `Anthropic`, and local-model adapter crates
- [x] Optional CLI linkage through independent Cargo feature flags
- [x] Vendor-specific JSON frame decoding and normalized protocol errors
- [x] Identical ordered partial-failure behavior across every adapter
- [x] Identical bounded-sink cancellation behavior across every adapter
- [x] Credential and prompt redaction at the injected transport boundary
- [x] Offline, credential-free cross-adapter contract suite
- [ ] Incremental SSE/HTTP transport (the current injected transport returns a
      bounded invocation batch)
- [ ] Local HTTP contract server covering status codes and streaming timing
- [ ] Provider-specific authentication headers and retry-after hints
- [ ] Opt-in nightly live tests with isolated quotas

Known limitation: these are real ABI adapters and vendor frame decoders, but
they do not yet initiate network or local-process I/O. The transport is an
injected capability exercised entirely offline; its current batch-returning
shape must be replaced with an incremental bounded frame stream before an HTTP
implementation can preserve end-to-end backpressure. Calling these production
HTTP adapters at this checkpoint would be inaccurate.

## Phase 7 exit gate

Not satisfied. The provider-neutral pipeline and four offline adapter contracts
pass, but the unchecked async-scope, failure-observability, cancellation-grace,
incremental transport, HTTP contract-server, and nightly-live-test items above
remain release blockers.
