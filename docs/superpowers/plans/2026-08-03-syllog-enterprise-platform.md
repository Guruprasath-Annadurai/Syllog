# Syllog Enterprise Language Platform Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver Syllog as a production programming-language platform with a cohesive framework experience: one project format, one CLI, fast development feedback, reproducible builds, secure runtimes, editor tooling, packages, native/Wasm targets, and stable enterprise releases.

**Architecture:** Preserve the current parser, semantic analyzer, streaming proxy, and Wasmtime sandbox as bootstrap components. Introduce explicit typed HIR and verified MIR boundaries, then lower MIR into Wasm first and native targets second. Build the framework experience above stable compiler and project APIs, keeping compiler correctness, runtime policy, package resolution, and developer tooling as separate crates with versioned interfaces.

**Tech Stack:** Rust 1.88+, Pest, Tokio, Wasmtime, wasm-encoder, Cranelift, LLVM behind an optional backend, Serde/TOML, Salsa-style incremental queries, Tower/LSP, Ed25519 signatures, OpenTelemetry, Cargo workspace tooling.

## Global Constraints

- Every production change follows red-green-refactor and lands with an independently meaningful test.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo build --workspace` are mandatory merge gates.
- Compiler and package builds must be deterministic for identical source, lockfile, compiler version, target, and feature set.
- Diagnostics retain stable codes, source spans, human output, and versioned JSON output.
- No garbage collector is introduced into the systems-language core; allocation and ownership behavior must remain explicit.
- No ambient filesystem, network, environment, clock, randomness, process, or native-host access is available to sandboxed code.
- `evo` and `asi_loop` remain disabled until Phase 12 gates are independently satisfied.
- Each numbered phase requires its own focused implementation plan before code execution; this document is the program-level roadmap.

---

## Product definition

The target workflow is intentionally as cohesive as a mature application framework:

```text
syllog new support-agent
cd support-agent
syllog dev
syllog check
syllog test
syllog build --target wasm32-syllog
syllog deploy --profile production
```

Generated project layout:

```text
support-agent/
├── Syllog.toml
├── Syllog.lock
├── src/
│   ├── main.syl
│   ├── agents/
│   ├── pipelines/
│   └── policies/
├── tests/
├── public/
└── .syllog/
    ├── cache/
    └── build/
```

The conventions are defaults, not hidden semantics. Every discovered source,
adapter, capability, build target, and deployment action must be inspectable in
`syllog inspect` output and overridable in `Syllog.toml`.

## Layered target architecture

```text
CLI / project framework / editor protocol
                 |
        incremental query engine
                 |
 source -> AST -> typed HIR -> verified MIR
                 |
        +--------+---------+
        |                  |
   Wasm backend       native backends
        |             Cranelift / LLVM
        +--------+---------+
                 |
 runtime ABI / capabilities / provider adapters
                 |
 deployment bundles / observability / policy
```

## Releases and indicative staffing

| Release | Phases | Product meaning | Indicative elapsed time with 8-10 engineers |
| --- | --- | --- | --- |
| Bootstrap baseline | 0-1 | Governed specification and trustworthy repository | 4-6 weeks |
| `0.1` executable core | 2-4 | Small typed programs compile and run as Wasm | 4-6 months |
| `0.3` framework alpha | 5-6 | `new/dev/build/test`, modules, packages, standard core | 7-10 months |
| `0.6` runtime/native beta | 7-9 | Production async, safety, native, and mobile foundations | 11-16 months |
| `0.8` ecosystem beta | 10 | Editor, formatter, documentation, and migration tooling | 15-19 months |
| `0.9` enterprise candidate | 11 | Security, operations, deployment, and release engineering | 18-23 months |
| `1.0` | 11 | Stable supported language and platform | 20-26 months |
| Research channel | 12 | Audited evolution experiments, isolated from stable | Only after 1.0 gates |

Minimum sustained team: compiler lead, two compiler engineers, runtime/security
engineer, framework/tooling engineer, LSP/IDE engineer, release/SRE engineer,
and test/specification engineer. Native/mobile and AI-provider work add two to
four specialists during their phases.

---

## Phase 0: Repository, governance, and reproducibility baseline

**Outcome:** Turn the current uncommitted prototype into a reviewable, repeatable engineering baseline.

### Task 0.1: Pin the toolchain and install mandatory CI gates

**Files:**
- Create: `rust-toolchain.toml`
- Create: `.github/workflows/ci.yml`
- Create: `deny.toml`
- Create: `docs/contributing.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: current Cargo workspace.
- Produces: one reproducible CI contract used by every later phase.

- [ ] Write a CI smoke script at `scripts/ci.sh` that exits nonzero when formatting, Clippy, tests, build, license policy, or checked-in examples fail.
- [ ] Run `bash scripts/ci.sh` and record the expected initial failure caused by missing dependency-policy configuration.
- [ ] Add the pinned stable Rust channel, Cargo-deny policy, CI cache keys based on `Cargo.lock`, and human/JSON `syllog check` example invocations.
- [ ] Run `bash scripts/ci.sh`; require exit code 0 on Linux and macOS runners.
- [ ] Commit as `chore: establish reproducible workspace gates`.

### Task 0.2: Establish compatibility and decision governance

**Files:**
- Create: `docs/governance/versioning.md`
- Create: `docs/governance/rfc-process.md`
- Create: `docs/governance/security.md`
- Create: `docs/adr/0001-compiler-pipeline.md`

**Interfaces:**
- Consumes: diagnostic schema v1 and draft LRM.
- Produces: RFC states, language edition rules, diagnostic compatibility rules, and disclosure process.

- [ ] Add a documentation test that validates all ADR/RFC files contain `Status`, `Decision`, `Compatibility`, and `Security impact` sections.
- [ ] Run `cargo test -p syllog-spec-tests governance_documents_are_complete` and observe failure before the validator exists.
- [ ] Implement the validator in the Phase 1 specification-test crate and add the four governance documents with explicit owners and review windows.
- [ ] Run the focused test and the workspace gate.
- [ ] Commit as `docs: define Syllog governance and compatibility policy`.

**Phase 0 exit gate:** Clean baseline commit, protected main branch, two-reviewer compiler/security ownership, CI green on supported hosts, no unpublished workspace changes.

---

## Phase 1: Normative specification and conformance system

**Outcome:** Make syntax and semantics testable independently from the compiler implementation.

### Task 1.1: Create the conformance harness

**Files:**
- Create: `crates/syllog-spec-tests/Cargo.toml`
- Create: `crates/syllog-spec-tests/src/lib.rs`
- Create: `crates/syllog-spec-tests/tests/conformance.rs`
- Create: `spec/cases/manifest.json`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces:
  ```rust
  pub struct ConformanceCase {
      pub edition: String,
      pub source: PathBuf,
      pub expected: ExpectedOutcome,
  }
  pub enum ExpectedOutcome {
      Pass,
      Diagnostics(Vec<String>),
      Run { stdout: String, exit_code: i32 },
  }
  pub fn load_cases(root: &Path) -> anyhow::Result<Vec<ConformanceCase>>;
  ```

- [ ] Write a failing loader test using literal pass and diagnostic fixtures.
- [ ] Run `cargo test -p syllog-spec-tests load_cases_preserves_expected_diagnostic_codes`; expect unresolved API failure.
- [ ] Implement manifest deserialization, normalized paths, duplicate-case rejection, and deterministic ordering.
- [ ] Run crate tests and the workspace gate.
- [ ] Commit as `test: add language conformance harness`.

### Task 1.2: Convert the LRM into normative executable cases

**Files:**
- Modify: `docs/language-reference.md`
- Create: `spec/cases/syntax/`
- Create: `spec/cases/types/`
- Create: `spec/cases/match/`
- Create: `spec/cases/agents/`
- Create: `spec/cases/pipelines/`

**Interfaces:**
- Consumes: `syllog_compiler::compile` and stable diagnostic codes.
- Produces: at least one positive and one negative fixture for each currently implemented grammar and semantic rule.

- [ ] Add one failing conformance test requiring every normative LRM rule identifier to map to at least one fixture.
- [ ] Run the test and observe missing rule identifiers.
- [ ] Assign stable identifiers such as `SYL-TYPE-OPTION-001` and add literal expected diagnostic lists.
- [ ] Run all conformance cases twice and assert identical ordered diagnostics.
- [ ] Commit as `spec: make current language rules executable`.

**Phase 1 exit gate:** The LRM clearly separates normative, experimental, and design-only features; all implemented normative rules have fixtures; diagnostic ordering is deterministic.

---

## Phase 2: Typed HIR and incremental compiler architecture

**Outcome:** Replace direct AST-to-analysis coupling with a stable, fully typed compiler representation.

### Task 2.1: Introduce stable identities and typed HIR

**Files:**
- Create: `crates/syllog-compiler/src/hir.rs`
- Create: `crates/syllog-compiler/src/lower.rs`
- Create: `crates/syllog-compiler/tests/hir_lowering.rs`
- Modify: `crates/syllog-compiler/src/lib.rs`
- Modify: `crates/syllog-semantic/src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct ModuleId(pub u32);
  pub struct DefId { pub module: ModuleId, pub index: u32 }
  pub struct HirProgram { pub modules: Vec<HirModule>, pub entry: Option<DefId> }
  pub struct TypedExpr { pub kind: HirExprKind, pub ty: ResolvedType, pub span: Span }
  pub fn lower_to_hir(ast: &Ast, symbols: &SymbolTable) -> Result<HirProgram, Vec<Diagnostic>>;
  ```

- [ ] Write failing tests proving every executable expression has a resolved type and every reference has a `DefId`.
- [ ] Run `cargo test -p syllog-compiler --test hir_lowering`; expect missing HIR types.
- [ ] Implement declaration indexing, reference lowering, typed expression lowering, and error sentinels without string-based lookups in HIR.
- [ ] Run HIR, semantic, and conformance suites.
- [ ] Commit as `feat: lower resolved programs into typed HIR`.

### Task 2.2: Add a query-oriented compilation database

**Files:**
- Create: `crates/syllog-compiler/src/database.rs`
- Create: `crates/syllog-compiler/tests/incremental.rs`
- Modify: `crates/syllog-compiler/src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct SourceFileId(pub u32);
  pub struct PackageId(pub u32);
  pub struct ParseResult { pub ast: Option<Ast>, pub diagnostics: Vec<CompilerDiagnostic> }
  pub struct HirResult { pub program: Option<HirProgram>, pub diagnostics: Vec<CompilerDiagnostic> }
  pub trait CompilerDatabase {
      fn set_source(&mut self, file: SourceFileId, text: Arc<str>);
      fn parse(&self, file: SourceFileId) -> Arc<ParseResult>;
      fn hir(&self, package: PackageId) -> Arc<HirResult>;
      fn diagnostics(&self, package: PackageId) -> Arc<[CompilerDiagnostic]>;
  }
  ```

- [ ] Write a failing test that edits one function body and asserts unrelated module parse results retain identity.
- [ ] Run the focused test and observe full recomputation.
- [ ] Implement revisioned source inputs, parse queries, dependency edges, cancellation checks, and deterministic diagnostic collection.
- [ ] Run the test under Loom-compatible synchronization checks where shared state is introduced.
- [ ] Commit as `feat: add incremental compiler database`.

**Phase 2 exit gate:** Typed HIR serializes in a versioned debug format, has no unresolved names in successful builds, supports cancellation, and passes incremental invalidation tests.

---

## Phase 3: Verified MIR and executable semantics

**Outcome:** Define the operational core once so interpreter, Wasm, and native backends agree.

### Task 3.1: Define control-flow MIR and verifier

**Files:**
- Create: `crates/syllog-ir/Cargo.toml`
- Create: `crates/syllog-ir/src/lib.rs`
- Create: `crates/syllog-ir/src/verify.rs`
- Create: `crates/syllog-ir/tests/verification.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces:
  ```rust
  pub struct BlockId(pub u32);
  pub struct LocalId(pub u32);
  pub enum MirType { Unit, Bool, I64, U64, Aggregate(DefId) }
  pub enum Operand { Constant(Constant), Copy(LocalId), Move(LocalId) }
  pub enum Place { Local(LocalId), Field { base: LocalId, field: u32 } }
  pub enum Statement { Assign { destination: Place, value: Rvalue }, Drop(Place) }
  pub struct MirProgram { pub functions: Vec<MirFunction> }
  pub struct MirFunction { pub blocks: Vec<BasicBlock>, pub locals: Vec<MirType> }
  pub struct BasicBlock { pub statements: Vec<Statement>, pub terminator: Terminator }
  pub enum Terminator { Return, Goto(BlockId), SwitchInt { value: Operand, targets: Vec<(u128, BlockId)>, otherwise: BlockId }, Call { function: DefId, args: Vec<Operand>, destination: Place, next: BlockId } }
  pub fn verify(program: &MirProgram) -> Result<(), Vec<MirVerificationError>>;
  ```

- [ ] Write failing verifier tests for missing terminators, invalid block targets, use-before-definition, and return-type mismatch.
- [ ] Run `cargo test -p syllog-ir`; expect unresolved verifier API.
- [ ] Implement MIR types and a verifier that rejects malformed internal programs before backend invocation.
- [ ] Run mutation-oriented tests that remove one terminator or corrupt one local type.
- [ ] Commit as `feat: define and verify Syllog MIR`.

### Task 3.2: Lower typed HIR to MIR

**Files:**
- Create: `crates/syllog-compiler/src/mir_lower.rs`
- Create: `crates/syllog-compiler/tests/mir_lowering.rs`

**Interfaces:**
- Consumes: `HirProgram`, `syllog_ir::MirProgram`.
- Produces: `pub fn lower_to_mir(hir: &HirProgram) -> Result<MirProgram, Vec<Diagnostic>>`.

- [ ] Write literal MIR-shape tests for constants, arithmetic, locals, calls, conditionals, enum construction, and `match`.
- [ ] Run the focused suite and observe missing lowering.
- [ ] Implement lowering with explicit temporaries, discriminants, block joins, and return destinations.
- [ ] Verify every produced MIR program before returning it.
- [ ] Commit as `feat: lower typed HIR into verified MIR`.

### Task 3.3: Add a reference MIR interpreter

**Files:**
- Create: `crates/syllog-interpreter/Cargo.toml`
- Create: `crates/syllog-interpreter/src/lib.rs`
- Create: `crates/syllog-interpreter/tests/execution.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces:
  ```rust
  pub enum RuntimeValue { Unit, Bool(bool), I64(i64), U64(u64), String(String), Aggregate(Vec<RuntimeValue>) }
  pub struct ExecutionResult { pub value: RuntimeValue, pub stdout: Vec<u8> }
  pub fn execute(program: &MirProgram, entry: DefId, limits: InterpreterLimits) -> Result<ExecutionResult, RuntimeError>;
  ```

- [ ] Write failing end-to-end cases for `fn main() -> I64 { 42 }`, calls, branching, and exhaustive enum matches.
- [ ] Run the cases and observe missing interpreter API.
- [ ] Implement deterministic MIR stepping with instruction, call-depth, allocation, and output limits.
- [ ] Compare interpreter results with literal expected values, never backend-generated expectations.
- [ ] Commit as `feat: add reference MIR interpreter`.

**Phase 3 exit gate:** A normative executable subset runs in the reference interpreter; malformed MIR cannot reach a backend; runtime semantics are represented by conformance cases.

---

## Phase 4: Wasm backend and genuine `syllog run`

**Outcome:** Ship the first complete source-to-execution vertical slice.

### Task 4.1: Lower verified MIR to WebAssembly

**Files:**
- Create: `crates/syllog-codegen-wasm/Cargo.toml`
- Create: `crates/syllog-codegen-wasm/src/lib.rs`
- Create: `crates/syllog-codegen-wasm/tests/codegen.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces:
  ```rust
  pub struct ArtifactMetadata { pub format_version: u32, pub entry: DefId, pub source_hash: [u8; 32] }
  pub struct WasmOptions { pub debug_info: bool, pub canonicalize_nan: bool }
  pub enum CodegenError { InvalidMir(Vec<MirVerificationError>), UnsupportedType(MirType), Encoding(String) }
  pub struct WasmArtifact { pub bytes: Vec<u8>, pub metadata: ArtifactMetadata }
  pub fn emit(program: &MirProgram, options: &WasmOptions) -> Result<WasmArtifact, CodegenError>;
  ```

- [ ] Write differential tests that execute the same MIR in the reference interpreter and Wasmtime and compare hand-derived results.
- [ ] Run the suite and observe missing backend API.
- [ ] Implement integer/Boolean/unit ABI, functions, locals, calls, branches, enum discriminants, and source-map custom sections.
- [ ] Validate every emitted module with `wasmtime::Module::validate` and execute it through `SandboxPolicy`.
- [ ] Commit as `feat: add verified MIR to Wasm backend`.

### Task 4.2: Make CLI build and run real

**Files:**
- Create: `crates/syllog-cli/src/commands/build.rs`
- Create: `crates/syllog-cli/src/commands/run.rs`
- Create: `crates/syllog-cli/tests/build_run.rs`
- Modify: `crates/syllog-cli/src/main.rs`

**Interfaces:**
- Produces commands:
  ```text
  syllog build FILE --target wasm32-syllog --output PATH
  syllog run FILE --fuel N --memory-bytes N
  ```

- [ ] Write a failing CLI test that builds and runs `fn main() -> I64 { 42 }`, asserting artifact existence, exit status, and clean stdout.
- [ ] Run the test and confirm current `run` only validates.
- [ ] Connect parse, resolve, HIR, MIR, Wasm emission, sandbox execution, and phased diagnostics.
- [ ] Add failure tests for missing `main`, invalid signature, fuel exhaustion, and memory denial.
- [ ] Commit as `feat: make syllog build and run executable`.

**Phase 4 exit gate / release `0.1`:** `syllog run` executes the documented core subset; interpreter and Wasm results agree across conformance cases; artifacts are deterministic and source-mapped.

---

## Phase 5: Next.js-class project and development experience

**Outcome:** Turn compiler commands into one coherent application framework.

### Task 5.1: Add project manifests and discovery

**Files:**
- Create: `crates/syllog-project/Cargo.toml`
- Create: `crates/syllog-project/src/manifest.rs`
- Create: `crates/syllog-project/src/discover.rs`
- Create: `crates/syllog-project/tests/projects.rs`
- Create: `docs/reference/manifest.md`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces:
  ```rust
  pub struct Manifest { pub package: Package, pub targets: Vec<Target>, pub dependencies: DependencyMap, pub capabilities: CapabilityProfile }
  pub fn discover(start: &Path) -> Result<Project, ProjectError>;
  pub fn load_manifest(path: &Path) -> Result<Manifest, Vec<ManifestDiagnostic>>;
  ```

- [ ] Write failing tests for parent-directory discovery, unknown keys, duplicate targets, capability profiles, and normalized paths.
- [ ] Run the focused tests and observe missing project crate.
- [ ] Implement strict `Syllog.toml` parsing with spans and JSON-compatible diagnostics.
- [ ] Add a schema export command for editor validation.
- [ ] Commit as `feat: add Syllog project manifests`.

### Task 5.2: Implement deterministic scaffolding

**Files:**
- Create: `crates/syllog-cli/src/commands/new.rs`
- Create: `crates/syllog-templates/basic/`
- Create: `crates/syllog-cli/tests/new_project.rs`

**Interfaces:**
- Produces: `syllog new NAME [--template basic|agent|native]`.

- [ ] Write a failing integration test that scaffolds into a temporary directory and immediately runs `syllog check` and `syllog test` there.
- [ ] Run it and observe the unknown `new` command.
- [ ] Implement validated package names, atomic directory creation, template version recording, and refusal to overwrite non-empty targets.
- [ ] Snapshot the generated manifest and source, then execute their behavior rather than testing template source strings alone.
- [ ] Commit as `feat: add deterministic project scaffolding`.

### Task 5.3: Implement incremental development mode

**Files:**
- Create: `crates/syllog-dev-server/Cargo.toml`
- Create: `crates/syllog-dev-server/src/lib.rs`
- Create: `crates/syllog-cli/src/commands/dev.rs`
- Create: `crates/syllog-dev-server/tests/rebuild.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum DevEvent { Building(BuildId), Diagnostics(EditorReport), Ready(ArtifactId), RuntimeRestarted }
  pub async fn serve(project: Project, options: DevOptions) -> Result<DevHandle, DevError>;
  ```

- [ ] Write a failing test that edits one source file and receives one debounced diagnostic/rebuild sequence without recompiling an unrelated module.
- [ ] Run it and observe missing dev-server API.
- [ ] Implement filesystem watching, incremental queries, cancellation of stale builds, structured events, and graceful shutdown.
- [ ] Add terminal rendering and `--json-events` without mixing machine output with logs.
- [ ] Commit as `feat: add incremental syllog dev workflow`.

### Task 5.4: Add test and inspect commands

**Files:**
- Create: `crates/syllog-cli/src/commands/test.rs`
- Create: `crates/syllog-cli/src/commands/inspect.rs`
- Create: `crates/syllog-cli/tests/test_inspect.rs`

**Interfaces:**
- Produces `syllog test`, `syllog inspect project`, `syllog inspect hir`, and `syllog inspect capabilities`.

- [ ] Write failing tests for source-level test discovery, deterministic ordering, capability explanation, and nonzero failure status.
- [ ] Run the focused CLI suite.
- [ ] Implement explicit `#[test]` metadata in AST/HIR, isolated sandbox execution, and stable JSON reports.
- [ ] Ensure inspection redacts secrets and displays convention-derived configuration.
- [ ] Commit as `feat: add project tests and inspection`.

**Phase 5 exit gate:** A new user can create, check, test, develop, build, run, and inspect a project through one CLI with sub-second no-op rebuilds on the reference project.

---

## Phase 6: Modules, packages, lockfiles, and standard core

**Outcome:** Support maintainable multi-package applications with reproducible dependencies.

### Task 6.1: Implement modules and visibility

**Files:**
- Modify: `crates/syllog-parser/src/grammar.pest`
- Modify: `crates/syllog-parser/src/ast.rs`
- Create: `crates/syllog-semantic/src/modules.rs`
- Create: `crates/syllog-semantic/tests/modules.rs`

**Interfaces:**
- Produces `module`, `use`, `pub`, module-qualified `DefId`, and cycle diagnostics.

- [ ] Write failing cross-file tests for imports, aliases, private access, duplicate exports, and dependency cycles.
- [ ] Run parser and semantic focused suites.
- [ ] Implement module graph construction before name resolution and preserve source spans across files.
- [ ] Add incremental invalidation tests for public-signature versus private-body changes.
- [ ] Commit as `feat: add multi-file modules and visibility`.

### Task 6.2: Implement the package resolver and lockfile

**Files:**
- Create: `crates/syllog-package/Cargo.toml`
- Create: `crates/syllog-package/src/resolver.rs`
- Create: `crates/syllog-package/src/lockfile.rs`
- Create: `crates/syllog-package/tests/resolution.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn resolve(manifest: &Manifest, index: &dyn PackageIndex, policy: ResolvePolicy) -> Result<Resolution, ResolveError>;
  pub fn write_lockfile(path: &Path, resolution: &Resolution) -> Result<(), LockfileError>;
  ```

- [ ] Write failing resolver fixtures for exact versions, compatible ranges, conflicts, yanked releases, checksums, offline mode, and malicious path traversal.
- [ ] Run the tests against an in-memory package index.
- [ ] Implement deterministic PubGrub-style resolution, content-addressed cache entries, checksums, and atomic lockfiles.
- [ ] Resolve the same graph under randomized index ordering and assert byte-identical lockfiles.
- [ ] Commit as `feat: add reproducible package resolution`.

### Task 6.3: Ship `core` and capability-oriented standard libraries

**Files:**
- Create: `library/core/`
- Create: `library/alloc/`
- Create: `library/io/`
- Create: `library/async/`
- Create: `library/provider/`
- Create: `spec/cases/library/`

**Interfaces:**
- Produces stable `core` types and opt-in libraries whose I/O entry points require explicit capability handles.

- [ ] Write conformance cases for every public function and failure contract before implementing library bodies.
- [ ] Run conformance cases and observe unresolved library symbols.
- [ ] Implement `core` without host dependencies, then layer allocation, I/O, async, and providers behind declared capabilities.
- [ ] Generate API documentation and public-symbol compatibility snapshots.
- [ ] Commit each library as an independently reviewable change.

### Task 6.4: Add registry, add, vendor, and publish workflows

**Files:**
- Create: `crates/syllog-registry-client/`
- Create: `crates/syllog-cli/src/commands/add.rs`
- Create: `crates/syllog-cli/src/commands/vendor.rs`
- Create: `crates/syllog-cli/src/commands/publish.rs`
- Create: `crates/syllog-registry-client/tests/contract.rs`

**Interfaces:**
- Produces `syllog add NAME@RANGE`, `syllog vendor`, and `syllog publish --dry-run`; registry requests and responses use a versioned protocol with content hashes and signed publisher identity.

- [ ] Write a local registry contract server and failing tests for immutable versions, checksum mismatch, namespace authorization, yanked downloads, offline vendoring, and publish replay.
- [ ] Run the focused suite without external network access.
- [ ] Implement resumable content-addressed downloads, authenticated publication, archive path validation, provenance upload, and atomic manifest/lockfile edits.
- [ ] Run published fixtures through a clean offline build from both cache and vendor directory.
- [ ] Commit as `feat: add secure package registry workflows`.

**Phase 6 exit gate / release `0.3`:** Multi-package projects build offline from a lockfile or vendor directory; signed package publication and retrieval pass local registry contracts; public/private modules work; the standard core has conformance and compatibility coverage.

---

## Phase 7: Production async and agent runtime

**Outcome:** Turn the current single-provider slice into structured, observable, policy-controlled orchestration.

### Task 7.1: Lower async functions into explicit state machines

**Files:**
- Create: `crates/syllog-compiler/src/async_lower.rs`
- Create: `crates/syllog-compiler/tests/async_lowering.rs`
- Modify: `crates/syllog-ir/src/lib.rs`
- Modify: `crates/syllog-runtime/src/lib.rs`

**Interfaces:**
- Produces MIR `Suspend`, `Resume`, and `Cancel` transitions plus an executor ABI whose tasks have explicit parent scope, wake handle, and drop path.

- [ ] Write failing tests for one await, multiple awaits, borrowed-local rejection across suspension, cancellation drops, panic propagation, and child-task scope exit.
- [ ] Run compiler/runtime focused suites and observe missing async MIR transitions.
- [ ] Lower each async function into a tagged state machine with verified live-local storage and one terminal drop path.
- [ ] Execute state machines with a deterministic test scheduler, then with Tokio, and require identical hand-derived event order.
- [ ] Commit as `feat: lower async functions into verified state machines`.

### Task 7.2: Define the versioned provider ABI and registry

**Files:**
- Split: `crates/syllog-proxy/src/lib.rs` into `provider.rs`, `registry.rs`, `router.rs`, `stream.rs`
- Create: `crates/syllog-proxy/tests/registry.rs`

**Interfaces:**
- Produces:
  ```rust
  pub trait ProviderAdapter: Send + Sync {
      fn descriptor(&self) -> &ProviderDescriptor;
      fn stream(&self, request: ModelRequest, sink: TokenSink) -> ProviderFuture<'_>;
  }
  pub struct ProviderRegistry;
  pub fn resolve(&self, route: &ModelRoute) -> Result<Arc<dyn ProviderAdapter>, ProviderLookupError>;
  ```

- [ ] Write failing tests for adapter version mismatch, duplicate registration, unknown provider, cancellation, and secret redaction.
- [ ] Run the proxy suite.
- [ ] Implement immutable registry snapshots, version negotiation, typed credentials, and cancellation-safe token sinks.
- [ ] Preserve bounded backpressure and ordered terminal errors.
- [ ] Commit as `feat: add versioned provider registry`.

### Task 7.3: Add structured pipeline execution

**Files:**
- Create: `crates/syllog-runtime/src/pipeline.rs`
- Create: `crates/syllog-runtime/tests/pipeline_execution.rs`

**Interfaces:**
- Produces `PipelinePlan`, bounded `Stage`, `JoinPolicy`, `RetryPolicy`, `CircuitPolicy`, and `PipelineHandle::cancel()`.

- [ ] Write deterministic paused-time tests for serial stages, bounded fan-out, merge ordering, cancellation, deadlines, retries, and circuit transitions.
- [ ] Run with Tokio’s paused clock and observe missing executor.
- [ ] Implement task scopes so child tasks cannot outlive their pipeline unless attached to an explicit supervisor.
- [ ] Emit structured lifecycle events without token content unless a logging capability permits it.
- [ ] Commit as `feat: execute structured streaming pipelines`.

### Task 7.4: Add real provider adapters behind feature flags

**Files:**
- Create: `crates/syllog-provider-openai/`
- Create: `crates/syllog-provider-anthropic/`
- Create: `crates/syllog-provider-local/`
- Create: `tests/provider-contract/`

**Interfaces:**
- Consumes: versioned `ProviderAdapter` contract.
- Produces adapters with identical cancellation, timeout, error-category, and token-order behavior.

- [ ] Build a local HTTP contract server covering success, partial stream failure, malformed frames, rate limits, timeout, and cancellation.
- [ ] Run each adapter against the contract server; no live vendor credentials are used in CI.
- [ ] Implement authentication through secret capability handles, SSE/HTTP streaming, normalized errors, and retry hints.
- [ ] Add opt-in nightly live tests with isolated quotas and secret-redacted output.
- [ ] Commit each adapter separately.

**Phase 7 exit gate:** Pipelines are bounded, cancellable, deadline-aware, observable, and provider-neutral; adapter contract tests pass without external services.

---

## Phase 8: Ownership, effects, and capability-aware type safety

**Outcome:** Fulfil the systems-language safety claim before native production use.

### Task 8.1: Implement affine moves and borrow analysis

**Files:**
- Create: `crates/syllog-semantic/src/ownership.rs`
- Create: `crates/syllog-semantic/tests/ownership.rs`
- Modify: `crates/syllog-ir/src/lib.rs`

**Interfaces:**
- Produces ownership states `Available`, `Moved`, `SharedBorrowed`, `MutBorrowed`, and region diagnostics tied to MIR locations.

- [ ] Write failing cases for use-after-move, double move, aliasing mutable references, borrow escape, branch joins, and valid reborrows.
- [ ] Run the ownership suite and observe acceptance of invalid cases.
- [ ] Implement move-path analysis, region constraints, control-flow joins, and drop insertion.
- [ ] Differentially execute accepted programs under an instrumented interpreter that detects double drops.
- [ ] Commit as `feat: enforce affine ownership and borrowing`.

### Task 8.2: Implement effect and capability checking

**Files:**
- Create: `crates/syllog-semantic/src/effects.rs`
- Create: `crates/syllog-semantic/tests/effects.rs`
- Modify: `crates/syllog-compiler/src/hir.rs`

**Interfaces:**
- Produces effect sets such as `pure`, `alloc`, `async`, `io`, `network`, `provider`, and `unsafe_ffi`; runtime capability requirements are emitted into artifact metadata.

- [ ] Write failing tests for undeclared effects, capability leakage, pure-function impurity, and effect propagation through calls.
- [ ] Run the focused semantic suite.
- [ ] Implement effect inference, explicit bounds at public APIs, and artifact capability manifests.
- [ ] Verify the runtime rejects artifacts whose declared requirements exceed deployment policy.
- [ ] Commit as `feat: type-check effects and runtime capabilities`.

**Phase 8 exit gate:** Safe Syllog rejects tested ownership violations; every host effect has a static declaration and runtime capability check.

---

## Phase 9: Native backends, FFI, and mobile foundations

**Outcome:** Produce supported native artifacts without weakening safety or reproducibility.

### Task 9.1: Define a backend-neutral ABI and Cranelift backend

**Files:**
- Create: `crates/syllog-abi/`
- Create: `crates/syllog-codegen-cranelift/`
- Create: `crates/syllog-codegen-tests/`

**Interfaces:**
- Produces target data layout, symbol mangling, enum representation, calling convention, panic ABI, and backend trait:
  ```rust
  pub struct TargetSpec { pub triple: String, pub pointer_width: u8, pub endian: Endian }
  pub struct CodegenOptions { pub optimization: OptimizationLevel, pub debug_info: bool }
  pub struct Artifact { pub bytes: Vec<u8>, pub metadata: ArtifactMetadata }
  pub enum CodegenError { InvalidMir(Vec<MirVerificationError>), UnsupportedTarget(String), Backend(String) }
  pub trait CodegenBackend {
      fn emit(&self, program: &MirProgram, target: &TargetSpec, options: &CodegenOptions) -> Result<Artifact, CodegenError>;
  }
  ```

- [ ] Write cross-backend ABI fixtures and differential execution cases before native emission.
- [ ] Run them and observe missing native backend.
- [ ] Implement host-native Cranelift emission for the stable core subset and link through a controlled driver.
- [ ] Compare Wasm, interpreter, and native results on every executable conformance case.
- [ ] Commit as `feat: add backend-neutral ABI and Cranelift target`.

### Task 9.2: Add LLVM release backend and target matrix

**Files:**
- Create: `crates/syllog-codegen-llvm/`
- Create: `targets/*.json`
- Create: `.github/workflows/targets.yml`

- [ ] Write target-layout tests for Linux, macOS, Windows, iOS, and Android triples.
- [ ] Run on the target CI matrix and observe unsupported backend errors.
- [ ] Implement LLVM lowering behind a feature flag, deterministic linker arguments, debug info, and optimization profiles.
- [ ] Execute native smoke binaries on hosted targets and cross-compile mobile libraries.
- [ ] Commit as `feat: add LLVM release backend and target specifications`.

### Task 9.3: Ship generated C, Swift, and Kotlin bindings

**Files:**
- Create: `crates/syllog-bindgen/`
- Create: `tests/ffi/c/`
- Create: `tests/ffi/swift/`
- Create: `tests/ffi/kotlin/`
- Create: `docs/reference/ffi.md`

- [ ] Write round-trip ABI tests for scalars, borrowed buffers, owned buffers, tagged unions, errors, and async callbacks.
- [ ] Run them before binding generation exists.
- [ ] Implement `extern` validation, header/module generation, ownership annotations, and panic containment.
- [ ] Run sanitizers and platform ABI tests; reject unsupported types at compile time.
- [ ] Commit as `feat: add safe generated native bindings`.

### Task 9.4: Add native UI host bindings without a JavaScript bridge

**Files:**
- Create: `crates/syllog-ui/`
- Create: `platform/apple/SyllogUI/`
- Create: `platform/android/syllog-ui/`
- Create: `tests/ui-contract/`
- Create: `docs/reference/native-ui.md`

**Interfaces:**
- Produces a retained, keyed `UiNode` tree, typed events, immutable render snapshots, and host adapters that map nodes to SwiftUI/UIKit and Compose/View primitives through generated bindings.

- [ ] Write platform-neutral failing reconciliation tests for keyed insertion, removal, reorder, state preservation, event coalescing, and accessibility metadata.
- [ ] Run contract tests before host adapters exist, then add simulator/emulator smoke tests for rendering and input dispatch.
- [ ] Implement deterministic tree diffing, main-thread host commits, bounded event queues, and zero-copy borrowed text/image buffers where platform lifetime rules permit them.
- [ ] Profile a 120 Hz update fixture and require no unbounded allocations, stale callbacks, or cross-thread UI access.
- [ ] Commit as `feat: add native UI tree host bindings`.

**Phase 9 exit gate / release `0.6`:** Wasm and native backends agree with the reference interpreter; supported mobile libraries build in CI; FFI ownership contracts pass sanitizers.

---

## Phase 10: Editor, formatter, documentation, and ecosystem tooling

**Outcome:** Provide the daily usability expected from an enterprise framework.

### Task 10.1: Build an incremental language server

**Files:**
- Create: `crates/syllog-lsp/`
- Create: `crates/syllog-lsp/tests/protocol.rs`
- Create: `editors/vscode/`

**Interfaces:**
- Produces diagnostics, hover, definition, references, rename, completion, semantic tokens, inlay hints, and code actions using the compiler database.

- [ ] Write protocol transcript tests with UTF-16 position conversion and cancellation.
- [ ] Run them against the absent server.
- [ ] Implement LSP framing and query-backed features without reparsing independently from the compiler.
- [ ] Add crash isolation, request deadlines, memory telemetry, and golden protocol snapshots.
- [ ] Commit as `feat: add incremental Syllog language server`.

### Task 10.2: Build a canonical formatter

**Files:**
- Create: `crates/syllog-format/`
- Create: `crates/syllog-format/tests/idempotence.rs`
- Create: `crates/syllog-cli/src/commands/fmt.rs`

- [ ] Write failing tests proving parse equivalence, comment preservation, idempotence, and stable line endings.
- [ ] Run property tests over generated valid ASTs.
- [ ] Implement lossless syntax/token preservation and width-configurable pretty printing.
- [ ] Require formatting a formatted file to produce byte-identical output.
- [ ] Commit as `feat: add canonical syllog fmt`.

### Task 10.3: Build versioned documentation and migration tooling

**Files:**
- Create: `crates/syllog-doc/`
- Create: `crates/syllog-migrate/`
- Create: `docs/editions/`

- [ ] Write failing tests for public API extraction, intra-doc links, doctest execution, and edition migration idempotence.
- [ ] Run them before generators exist.
- [ ] Implement documentation from typed HIR and syntax-aware edition migrations.
- [ ] Execute every documentation example as a conformance case.
- [ ] Commit as `feat: add API docs and edition migrations`.

**Phase 10 exit gate:** Editing, navigation, formatting, docs, diagnostics, and migrations share compiler truth; no tool maintains a competing parser or type model.

---

## Phase 11: Enterprise hardening and 1.0 release

**Outcome:** Establish operational trust, compatibility, and supported delivery.

### Task 11.1: Secure supply chain and reproducible artifacts

**Files:**
- Create: `crates/syllog-artifact/`
- Create: `docs/security/artifacts.md`
- Create: `.github/workflows/release.yml`

- [ ] Write tests that alter source, lockfile, compiler, target, capability manifest, or dependency checksum and require a different artifact identity.
- [ ] Run them before artifact manifests exist.
- [ ] Implement content-addressed builds, SBOM generation, provenance attestations, Ed25519 signatures, and verification before execution/deployment.
- [ ] Rebuild releases on independent workers and require byte-identical artifacts where platform linkers permit it.
- [ ] Commit as `security: sign and attest reproducible artifacts`.

### Task 11.2: Add observability and operational policy

**Files:**
- Create: `crates/syllog-telemetry/`
- Create: `docs/operations/`
- Create: `tests/operations/`

- [ ] Write tests for trace propagation, metric cardinality limits, secret/token redaction, audit integrity, and telemetry-disabled mode.
- [ ] Run them before telemetry integration.
- [ ] Implement OpenTelemetry-compatible spans for builds, pipelines, providers, and sandboxes with policy-controlled payload capture.
- [ ] Load-test bounded queues and verify telemetry cannot block token or compiler progress.
- [ ] Commit as `feat: add policy-controlled runtime observability`.

### Task 11.3: Implement policy-controlled deployment

**Files:**
- Create: `crates/syllog-deploy/`
- Create: `crates/syllog-cli/src/commands/deploy.rs`
- Create: `crates/syllog-deploy/tests/bundles.rs`
- Create: `docs/reference/deployment.md`

**Interfaces:**
- Produces `syllog deploy --profile NAME [--dry-run]`, immutable `DeploymentBundle`, `DeploymentPlan`, and a versioned `DeploymentAdapter` trait; the first adapter emits an OCI-compatible Wasm bundle without contacting a remote control plane during tests.

- [ ] Write failing tests for profile resolution, capability-policy narrowing, signed artifact verification, secret references, dry-run purity, idempotency keys, and rollback plan generation.
- [ ] Run the deployment suite against a local fake registry and assert no ambient network access.
- [ ] Implement bundle assembly from signed artifacts and lockfiles, policy validation, SBOM attachment, adapter protocol, and machine-readable deployment events.
- [ ] Deploy and roll back a local Wasmtime service fixture twice, proving the second identical deployment is a no-op.
- [ ] Commit as `feat: add policy-controlled deployment bundles`.

### Task 11.4: Run compatibility, performance, and resilience gates

**Files:**
- Create: `benchmarks/`
- Create: `compat/`
- Create: `fuzz/`
- Create: `docs/releases/1.0-readiness.md`

- [ ] Establish checked-in benchmark hardware/configuration metadata and baselines for cold check, incremental check, codegen, startup, memory, token throughput, and sandbox overhead.
- [ ] Add parser, AST lowering, semantic, MIR verifier, package, formatter, and Wasm-import fuzz targets with minimized regression corpora.
- [ ] Test previous compiler/project/artifact compatibility according to the versioning policy and rehearse rollback of a signed release.
- [ ] Require zero known critical security findings, no unresolved soundness defects, and documented performance-regression approvals.
- [ ] Commit as `release: satisfy Syllog 1.0 readiness gates`.

**Phase 11 exit gate / release `1.0`:** Stable language edition, documented support matrix, reproducible signed releases, compatibility promise, vulnerability response process, operational runbooks, and measured service-level objectives.

---

## Phase 12: Gated `evo` and `asi_loop` research channel

**Outcome:** Permit controlled experiments only after the stable platform can prove provenance, isolation, rollback, and policy compliance.

### Gate 12.1: Evolution artifact lifecycle

Required before any hot-swap implementation:

- Signed input source and dependency provenance.
- Deterministic compilation into the stable Wasm ABI.
- Static capability manifest no broader than the active module.
- Independent policy approval and safety-proof verification.
- Shadow execution, bounded canary, health criteria, atomic promotion, and tested rollback.
- Append-only audit record linking proposal, proof, artifact hash, approver, deployment, and outcome.

### Gate 12.2: Autonomous-loop containment

Required before any `asi_loop` scheduler implementation:

- Externally controlled iteration, wall-time, fuel, memory, network, storage, and monetary budgets.
- No authority for a loop to expand its own capabilities, modify its verifier, erase audit history, or promote its own artifact.
- Independent stop controller and operator pause/revoke path.
- Reproducible evaluation datasets with contamination controls.
- Adversarial tests for reward hacking, evaluator tampering, covert channels, rollback failure, and concurrent promotion races.

### Phase 12 implementation rule

Create separate RFCs and repositories or workspace feature channels for `evo`
and `asi_loop`. Stable builds reject these constructs unless an explicitly
experimental compiler channel is selected. No Phase 12 feature can be required
by core language, package, build, editor, native, or provider functionality.

---

## Cross-phase engineering controls

### Definition of done for every task

- A focused test was observed failing for the intended missing behavior.
- Minimal implementation made the focused test pass.
- Existing conformance and workspace tests pass.
- Human and JSON errors remain actionable and deterministic.
- Public interfaces have documentation and compatibility impact recorded.
- Security/capability impact is recorded when host effects change.
- Benchmark impact is recorded after Phase 4.
- The change is committed independently with no unrelated user work included.

### Quality budgets

| Dimension | Required gate |
| --- | --- |
| Compiler correctness | Differential interpreter/backend tests and conformance fixtures |
| Incremental correctness | Clean and incremental builds produce identical diagnostics/artifacts |
| Diagnostics | Stable code, exact span, remediation text, JSON schema compatibility |
| Runtime memory | Every unbounded queue/allocation requires an explicit policy owner |
| Security | Deny-by-default capabilities; signed release provenance; fuzzing at parser and artifact boundaries |
| Reliability | Cancellation, timeout, shutdown, and partial-failure tests for every async subsystem |
| Compatibility | Edition and semantic-version policy enforced by automated snapshots |
| Performance | Reproducible benchmarks with reviewed regression thresholds |

### Program risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Attempting the entire vision before executable basics | Phases 2-4 exclusively prioritize the small source-to-Wasm vertical slice. |
| Three backends diverge semantically | Reference interpreter plus differential conformance tests are authoritative. |
| Framework convenience hides capabilities | `syllog inspect capabilities` exposes every derived requirement. |
| AI providers destabilize the core language | Versioned adapter ABI and external provider crates isolate vendor behavior. |
| Incremental compiler creates stale results | Clean/incremental equivalence tests and explicit dependency queries. |
| Native/mobile scope delays usability | Wasm is the first executable target; native follows stable MIR and ABI. |
| Self-modification bypasses governance | Phase 12 is isolated, deny-by-default, externally approved, and post-1.0. |

## Immediate next 90 days

1. Complete Phase 0 and commit the current bootstrap baseline.
2. Build Phase 1’s conformance harness and classify every LRM feature as implemented, experimental, or design-only.
3. Implement Phase 2 typed HIR with stable `DefId` references.
4. Begin Phase 3 MIR using constants, integer arithmetic, locals, calls, branches, enum discriminants, and match.
5. Demonstrate the first interpreter execution of `fn main() -> I64 { 42 }`.

Do not start package hosting, native UI, real provider credentials, `evo`, or
`asi_loop` during this window. The first organizational proof point is a clean,
typed, verified, executable core—not breadth.
