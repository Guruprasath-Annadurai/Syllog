# Phase 8 implementation ledger

Updated: 2026-08-09

Checked items have executable tests in this repository. This ledger separates
the safe subset now enforced from the work required before Syllog may claim
Rust-level memory safety or enterprise launch readiness.

## Gate 8.1 — Affine ownership and moves

- [x] Non-`Copy` locals and parameters move on assignment, calls, and returns
- [x] Source diagnostics reject use-after-move and moves while borrowed
- [x] Branch joins conservatively retain a move made on any match arm
- [x] MIR rejects use-after-move, `Copy` of affine values, and double drops
- [x] Compiler inserts deterministic drops for owned values live at exits
- [x] The instrumented reference interpreter counts successful drop execution
- [ ] Field-sensitive partial moves and independently tracked aggregate fields
- [ ] Loop fixed-point cases at the source ownership layer
- [ ] Linear `@must_use` resources that require exactly-once consumption
- [ ] Ownership typing for payload bindings in executable match patterns

The source checker currently treats a borrowed field as a borrow of the whole
root aggregate. This is safe but rejects programs a field-sensitive checker
could accept. MIR verification is a second boundary and rejects malformed
compiler or tool-generated ownership operations.

## Gate 8.2 — Borrowing and regions

- [x] Parsed and spanned `&T`, `&mut T`, `&'a T`, and `&'a mut T`
- [x] Multiple shared borrows and exclusive mutable borrow conflicts
- [x] Non-lexical release at the last use of a local reference binding
- [x] Local-reference escape rejection
- [x] Public return-region linkage to an input reference
- [x] Conservative rejection of live borrows crossing `await`
- [ ] Dereference, mutation-through-reference, and reference-valued field access
- [ ] General region constraint solving for nested types and reborrows
- [ ] Variance, subtyping, higher-ranked bounds, and closure captures
- [ ] `Send`, `Sync`, pinning, interior mutability, and thread-race proofs

The current Wasm/interpreter reference representation is an immutable checked
snapshot. Mutable alias creation is rejected, but mutation through a reference
is not yet a language operation. Borrow-across-await is rejected rather than
proved safe, which is conservative and intentional.

## Gate 8.3 — Effects and static capabilities

- [x] Source effect bounds: `pure`, `alloc`, `async`, `io`, `network`, `provider`
- [x] Direct inference for allocation and async suspension
- [x] Provider/network inference for agent-backed pipelines
- [x] Fixed-point transitive propagation through the function call graph
- [x] Unknown, duplicate, mixed-`pure`, and under-declared bounds are rejected
- [x] `syllog check` exposes a distinct `effect_check` diagnostic phase
- [x] Package-wide HIR receives one deterministic capability manifest
- [ ] Require explicit effect bounds on every exported function
- [ ] Effect-polymorphic functions and effect variables
- [ ] Source host intrinsics for every `io`, `network`, and `provider` operation
- [ ] `unsafe_ffi` effect; it remains deferred with native FFI to Phase 9
- [ ] Exact source filenames for effect errors from linked multi-file packages

The artifact manifest conservatively unions every compiled executable
definition, not only definitions reachable from `main`. That can over-grant but
cannot under-grant due to dead-code reachability mistakes.

## Gate 8.4 — Runtime agreement

- [x] Wasm artifacts embed a versioned `syllog.capabilities` custom section
- [x] The provenance hash commits to MIR, async frames, and capabilities
- [x] Wasmtime rejects required effects absent from `SandboxPolicy`
- [x] Duplicate, malformed, or unsupported manifests fail closed
- [x] Foreign Wasm remains governed by import, fuel, and memory policies
- [ ] Cryptographic signing of the final artifact manifest
- [ ] Deployment-policy files that independently grant CLI runtime effects
- [ ] Native backend enforcement equivalent to the Wasmtime boundary

`syllog run` grants the effects produced by the source it has just compiled.
Embedding applications can supply a narrower `SandboxPolicy`, and denial is
covered by integration tests. Independent deployment authorization belongs in
the enterprise hardening phase.

## Verification evidence

Focused suites cover parser spans, source ownership diagnostics, borrow escape,
borrow/await rejection, affine MIR corruption, generated drops, interpreter
drop counts, transitive effect inference, deterministic artifact metadata, and
runtime effect denial. On 2026-08-09, the full `scripts/ci.sh` gate passed:
formatting, Clippy with warnings denied, all workspace targets and tests, the
workspace build, dependency policy, and CLI compilation checks. Two opt-in live
provider tests remain intentionally ignored because they require credentials
and external quota.

## Phase 8 exit decision

**Not yet satisfied for a Rust-level memory-safety claim.** The implemented
subset prevents the tested affine and aliasing violations, but the unchecked
items above—especially complete move paths, general region solving,
reference mutation semantics, concurrency traits, and native/FFI validation—
are required before that claim is technically defensible.

The current subset is suitable for continued compiler and sandbox development;
it is not an enterprise production launch approval.
