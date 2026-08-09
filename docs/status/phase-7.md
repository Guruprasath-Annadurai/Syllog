# Phase 7 implementation ledger

Updated: 2026-08-09

Checked items have executable tests in this repository. A checked implementation
item is not a claim that an external provider accepted a live request; that
separate evidence is recorded below.

## Gate 7.1 — Async lowering and structured tasks

- [x] `await` syntax with `SYL2501` outside `async fn`
- [x] Verified `Start`, `Suspend`, `Resume`, `Complete`, `Cancel`, and `Panic`
      transitions
- [x] Conservative live-local frame metadata and one shared drop state
- [x] Cancellation and panic run the shared drop path exactly once
- [x] Wasm exports resumable step functions and embeds async-frame metadata
- [x] Pipeline fan-out uses a child scope that cancels and joins siblings
- [x] Currently representable borrowed `Str` parameters and locals are rejected
      when an async function contains an `await`
- [ ] General reference syntax, region inference, and a complete borrow checker
- [ ] Source-level `spawn` and arbitrary nested task scopes

The borrow gate is deliberately conservative. Syllog does not yet expose a
general `&T` reference type, so this check covers the borrowed string type that
the current type system can represent. The complete ownership and borrowing
model remains a Phase 8 requirement.

## Gate 7.2 — Versioned provider ABI

- [x] Explicit major/minor provider ABI descriptor
- [x] Incompatible ABI and duplicate registration rejection
- [x] Immutable registry snapshots and exact route lookup
- [x] Credential-kind declarations and redacted secret values
- [x] OpenAI bearer and Anthropic API-key/version headers
- [x] Provider requests keep credentials out of bodies and lifecycle events
- [x] Explicit idempotent cancellation with bounded sinks
- [x] Incremental transport frames preserve ordered partial failures
- [ ] OS keychain, workload identity, or HSM-backed credential sources

Credentials are capability-wrapped, excluded from formatting and
serialization, and marked sensitive at the HTTP layer. Environment-to-secret
injection and production secret rotation remain deployment responsibilities.

## Gate 7.3 — Production pipeline executor

- [x] Serial stage execution
- [x] Bounded fan-out with declaration-order and completion-order joins
- [x] Retry backoff, total stage deadlines, and shared circuit breakers
- [x] Structured failure values retain accumulated lifecycle events
- [x] Cooperative cancellation waits for child cleanup
- [x] Configurable cancellation grace period
- [x] Non-cooperative roots are explicitly recorded and reaped by a supervisor
- [x] Lifecycle events exclude pipeline payloads

Lifecycle retention is bounded by the configured event capacity. If that
capacity is exhausted, oldest events are evicted; production deployments must
size it from their maximum expected stage/attempt count or export events to an
external sink.

## Gate 7.4 — Provider transports and contracts

- [x] Separate OpenAI, Anthropic, and local-model adapter crates
- [x] Independent CLI feature flags
- [x] Incremental bounded HTTP/SSE decoding with downstream backpressure
- [x] Split-chunk, CRLF, multi-line, `[DONE]`, UTF-8, content-type, and size
      handling
- [x] Loopback HTTP contract server verifies realistic stream timing and
      mid-stream connection failure
- [x] Contract tests verify exact OpenAI and Anthropic authentication headers
- [x] HTTP 429 normalization preserves seconds or HTTP-date `Retry-After` hints
- [x] Bounded error bodies and normalized timeout/unavailable/auth failures
- [x] Redirects are rejected before credentials can cross endpoint boundaries
- [x] Real local child-process transport without a shell
- [x] Real loopback TCP socket transport with public-address rejection
- [x] One absolute deadline covers local connect/write/read/process completion
- [x] Offline credential-free cross-adapter contract suite
- [x] Nightly/manual opt-in live-provider workflow with isolated secret inputs
- [ ] Automatic provider retry scheduling from `Retry-After`
- [ ] A successfully executed live-provider workflow run in this checkout

The runtime surfaces `Retry-After` as typed error metadata so pipeline policy
can decide whether replay is safe. The HTTP adapter does not silently retry a
possibly non-idempotent request. Live tests are ignored unless
`SYLLOG_LIVE_PROVIDER_TESTS=1` and provider credentials/models are supplied;
they were not run during this implementation because no live credentials were
used.

## Verification evidence

Focused tests cover HTTP contracts, local process/socket I/O, lifecycle failure
retention and detachment, async borrow rejection, and Wasm state resumption. The
repository CI script is the final local gate for formatting, lockfile policy,
Clippy, unit/integration tests, docs, audit, and deny checks.

## Phase 7 exit gate

The repository implementation gate is satisfied when the full CI script passes
at the Phase 7 commit. External launch approval remains open until a controlled
live-provider run, load/fault testing, secret-store integration, and production
observability/SLO validation are completed. Those deployment gates must not be
inferred from green offline tests.
