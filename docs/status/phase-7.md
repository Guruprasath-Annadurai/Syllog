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

Status: not started.

## Gate 7.3 — Production pipeline executor

Status: not started.

## Gate 7.4 — Provider adapters

Status: not started.

## Phase 7 exit gate

Not satisfied. Provider-neutral bounded pipelines and all adapter contracts must
pass without external services before Phase 7 can be released.
