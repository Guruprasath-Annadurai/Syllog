# Phase 6 implementation ledger

Updated: 2026-08-09

This ledger records verified implementation state. A checked item means its
focused tests and the repository gate passed at its checkpoint; it does not
mean adjacent or later Phase 6 behavior exists.

## Gate 6.1 — Modules and visibility

- [x] `module` declarations and `use` imports with aliases are represented in
  the span-aware AST.
- [x] Module IDs and definition IDs are deterministic under input reordering.
- [x] Imported public functions participate in per-file body type analysis.
- [x] Private and unknown imports produce source-ranged diagnostics.
- [x] Duplicate definitions and local import collisions are rejected.
- [x] Dependency cycles produce a deterministic complete-cycle diagnostic.
- [x] Public-interface hashes ignore private bodies and source layout while
  changing for public function signature changes.
- [x] Positive and negative normative module-syntax fixtures exist.

`syllog build` and `syllog run` now accept a project directory or a declared
target and compile every `.syl` source below the target source root. Package HIR
assigns stable module-qualified identities, links cross-module calls, and merges
multiple files contributing to one logical module. Imported agent input/output
contracts and imported pipeline call signatures participate in type checking.

Known limitation: `syllog check` and the incremental dev server have not yet
been switched to the package compilation entry point. Source-root discovery is
currently convention-based from the target file's parent directory rather than
an explicit manifest source-root field.

## Gate 6.2 — Package resolver and lockfile

- [x] Deterministic backtracking version constraint solver
- [x] Yank and conflict handling with stable requirement provenance
- [x] Offline policy and immutable content-addressed cache
- [x] SHA-256 content verification on resolution and cache reads
- [x] Atomic deterministic `Syllog.lock`
- [x] Archive/path traversal defenses
- [x] Package-aware multi-file HIR/MIR build and Wasm linking
- [x] Deterministic minimum-remaining-values selection and failed-state memoization
- [x] 300-package/four-version deterministic solver stress contract

Checkpoint evidence: 11 focused adversarial tests pass, including input-order
permutations that produce byte-identical lockfiles; the full repository CI gate
passes. The solver backtracks correctly, prioritizes the smallest candidate set,
and memoizes failed states. It does not yet implement PubGrub-style learned
incompatibility explanations or a configurable resource budget, so adversarial
registry-scale worst-case complexity is not claimed.

## Gate 6.3 — Standard libraries

- [x] host-independent `core` bootstrap package
- [x] checked `alloc` bootstrap package
- [x] capability-parameterized `io` bootstrap package
- [x] capability-parameterized `async` bootstrap package
- [x] capability-parameterized `provider` bootstrap package
- [x] full-signature API compatibility snapshot and conformance coverage

Known limitation: these are deliberately minimal source-level bootstrap APIs.
They do not claim completed host I/O, allocation, scheduling, or provider
adapters; those implementations remain gated by Phases 7 and 9. Capability
tokens have private fields and every authority-bearing public function requires
an explicit token parameter.

## Gate 6.4 — Registry workflows

- [x] Versioned no-network local registry contract
- [x] Ed25519 publisher identity, namespace authorization, replay defense, and immutable releases
- [x] Atomic, comment-preserving `syllog add NAME@RANGE`
- [x] Checksum-verified offline `syllog vendor`
- [x] Deterministic `syllog publish --dry-run`
- [x] Offline rebuild from verified lockfile cache
- [x] Offline rebuild from a self-contained verified vendor directory
- [x] Lock graph, manifest requirement, archive metadata, and reachability validation
- [x] HTTPS registry client with resumable range downloads and strict checksum/identity verification
- [x] `syllog fetch --registry URL` for locked archive retrieval
- [x] Non-dry-run signed publication with atomic checksum-bound provenance upload

Checkpoint evidence: 7 registry security contracts, a real loopback HTTP range
and publication contract, and 6 end-to-end CLI package workflow tests pass under
strict Clippy. Bearer credentials are formatting-redacted, non-loopback registry
URLs require HTTPS, partial downloads validate `Content-Range`, and publication
receipts must match the signed request.

Known limitation: this repository contains the registry protocol and client,
not a horizontally scalable registry service. Lockfile creation is not yet
wired to remote index metadata, resumable uploads are not implemented, and
credentials currently enter through environment variables rather than an OS
keychain/HSM provider.

## Phase 6 exit gate

Core functional gate satisfied at this checkpoint: clean multi-package builds
execute from both a lockfile-backed cache and a cache-independent vendor
directory; signed network publication and resumable retrieval pass local HTTP
contracts.

Enterprise launch gate remains open. Required hardening: package-aware
`check/dev/test`, remote index-to-lock resolution, package namespace isolation,
resolver resource budgets and learned conflict explanations, resumable uploads,
OS keychain/HSM credentials, a production registry service, fault-injection and
load tests, and signed release operations.
