# Feature Status

This matrix describes commit-local evidence. “Partial” means at least one
required semantic, runtime, security, test, documentation, or distribution path
is missing. Phase ledgers provide deeper implementation notes but do not override
this summary.

## Implemented

| Capability | Evidence | Limits |
| --- | --- | --- |
| Span-aware Pest parsing | `syllog-parser`, parser tests | No partial-AST recovery |
| Diagnostics | human and JSON schema v1, compiler tests | No LSP UTF-16 adapter |
| Core name/type analysis | `syllog-semantic`, semantic tests | Accepted syntax exceeds executable subset |
| Verified core MIR | `syllog-ir`, corruption tests | Verification is structural/type validation, not formal proof |
| Reference execution | `syllog-interpreter`, conformance cases | Scalar/control-flow subset only |
| Wasm emission/execution | `syllog-codegen-wasm`, `syllog-runtime` tests | No native backend; capability gaps remain |
| Deterministic lockfile/cache/vendor primitives | package tests and Phase 6 ledger | No claim for all large conflict graphs or a production registry |

## Partially implemented

| Capability | What works | What is missing |
| --- | --- | --- |
| Ownership and borrowing | Selected affine moves, borrow conflicts, regions, drops | Full place/projection model, reborrows, closure captures, all control-flow joins, borrow safety across every `await` |
| Async execution | Tokio runtime infrastructure and async metadata/state-machine prototypes | General source `async` lowering and resumable Wasm semantics |
| Effects/capabilities | Effect inference, artifact manifest, deny-by-default host imports | User-declared least-authority grants, complete static/runtime equivalence, foreign-Wasm manifest policy |
| Agents and pipelines | Parsed/typed declarations; Rust provider/router primitives | Compiler lowering from declarations to production runtime graphs |
| Package ecosystem | Resolver, lockfile, cache, vendor, signed archives, dry-run publish | Hosted registry, resilient uploads, deployed provenance service, mature conflict solving |
| Standard library | Five versioned bootstrap packages (`core`, `alloc`, `io`, `async`, `provider`) parse/type-check and expose small data/validation APIs | Runtime-backed allocation, I/O, task scheduling, provider calls, documentation generation, and compatibility depth |
| Developer server | File watching, debouncing, rebuild events | Application runtime lifecycle, browser/mobile serving, stable incremental query engine |
| Deterministic execution | Deterministic core interpreter/Wasm output and mock provider | External providers, scheduling, clocks, network, and host I/O are not deterministic |
| Artifact provenance | Source hashes and package signatures/checksums in selected paths | Reproducible release pipeline, SBOM, signed attestations, verification UX |
| Cross-platform support | Rust workspace is intended for Linux/macOS/Windows CI; Wasm target exists | Green public three-OS evidence, native backends, FFI, mobile, UI |

## Planned

- Frozen, traceable Syllog 0.1 semantics across parser, HIR, MIR, interpreter,
  and Wasm.
- Formatter, LSP, editor extension, API docs, release binaries, checksums, SBOM,
  and provenance attestations.
- Production capability-safe agent graphs with typed tools, budgets, structured
  concurrency, cancellation, and auditable evidence.
- WebAssembly Component Model/WIT and stable host SDK boundaries.
- Post-1.0 research RFCs for governed evolution; `evo` and `asi_loop` are not
  implemented features.

## Claims policy

Syllog is experimental. It must not be described as production-ready,
enterprise-ready, Rust-memory-safe, formally verified, fully deterministic, or
secure merely because a partial mechanism exists. Each status promotion requires
a written contract, implementation, positive and negative tests, documented
failure behavior, runtime enforcement where applicable, and known limitations.
