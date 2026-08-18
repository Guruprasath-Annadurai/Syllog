# Current Architecture

This document maps the code that is built by the Cargo workspace. It is
descriptive, not an assertion that every crate is production complete.

## Compiler and execution flow

The authoritative compiler flow is the one defined in ADR 0001 and orchestrated
by `crates/syllog-compiler`: source → AST → typed HIR → verified MIR → interpreter
or Wasm. The runtime instantiates emitted Wasm in a fresh Wasmtime store with
configured fuel, memory, and host capabilities. Agent/provider runtime crates are
currently parallel infrastructure; `.syl` agent declarations are not yet lowered
into an executable provider graph.

## Workspace responsibilities

| Package | Responsibility | Current boundary |
| --- | --- | --- |
| `syllog-cli` | User-facing project, package, compiler, and execution commands | Several framework-style commands are absent |
| `syllog-parser` | Authoritative Pest grammar, AST, spans, parsing | Syntax errors stop AST production |
| `syllog-semantic` | Symbol tables, resolution, types, match checks, partial ownership analysis | Borrow/lifetime model is incomplete |
| `syllog-compiler` | Phase orchestration, HIR lowering, diagnostics | Executable subset is smaller than accepted syntax |
| `syllog-ir` | MIR data model and verifier | Internal format, not a proof system |
| `syllog-interpreter` | Deterministic reference execution for supported MIR | No external I/O/provider semantics |
| `syllog-codegen-wasm` | Deterministic Wasm emission from verified MIR | Core scalar/control-flow subset |
| `syllog-runtime` | Wasmtime execution with configured resource/capability policy | Arbitrary foreign Wasm requires operator care |
| `syllog-project` | Strict project manifests and discovery | Bootstrap schema |
| `syllog-package` | Resolver, lockfile, cache, and vendoring primitives | Large-graph and hosted-registry maturity incomplete |
| `syllog-registry-client` | Signed archive and HTTP registry protocol prototype | No production registry service or publish deployment |
| `syllog-dev-server` | Debounced file watching and incremental rebuild events | It is not an application runtime or web server |
| `syllog-proxy` | Tokio streams, routing, retries, circuits, cancellation prototypes | Not connected to compiled `.syl` pipelines |
| `syllog-provider-openai` | Feature-gated OpenAI adapter | Live testing is opt-in and credential-dependent |
| `syllog-provider-anthropic` | Feature-gated Anthropic adapter | Live testing is opt-in and credential-dependent |
| `syllog-provider-local` | Feature-gated local HTTP adapter | Process/socket production transport incomplete |
| `syllog-spec-tests` | Governance and language conformance contracts | Corpus, property, and fuzz depth are incomplete |
| `syllog-provider-contract-tests` | Offline and opt-in live adapter contracts | Live tests are ignored by default |

## Trust boundaries

- `.syl`, manifests, lockfiles, package archives, provider responses, and Wasm
  modules are untrusted inputs.
- The parser/semantic pipeline must reject invalid inputs without panicking.
- MIR must pass the verifier before interpretation or code generation.
- Package hashes/signatures authenticate bytes, not safety or maintainer intent.
- Capability manifests and runtime linkers must agree; current gaps are recorded
  in the feature matrix and runtime documentation.
- Provider credentials belong only in process environment/secret infrastructure
  and must never enter source, artifacts, diagnostics, or snapshots.

## Historical code

Root `src/` contains the original hand-written prototype. Root `Cargo.toml` has
only a workspace table and no package, so Cargo does not build this directory.
The directory is frozen pending a separately approved archival/removal decision.
