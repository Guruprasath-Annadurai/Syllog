# Syllog Language Reference Manual

Version 0.1-draft · 3 August 2026

## 1. Status and conformance

This document defines the intended Syllog v1 language. Normative terms **must**,
**must not**, **should**, and **may** have their RFC 2119 meanings. Features are
classified as follows:

- **Executable normative** rules are listed by stable identifier in §14.1 and
  are enforced by the current compiler and conformance suite.
- **Design-only** material describes the intended v1 contract but is not an
  implemented language promise until promoted into §14.1 by an accepted RFC and
  executable positive and negative fixtures.
- **Experimental** material covers `probe`, `zk_verify`, `evo`, and `asi_loop`.
  It is post-1.0 research, unavailable in production profiles, and cannot be
  used to claim language conformance.

Sections 2–11 are design-only except where a rule is explicitly promoted in
§14.1. Sections 12–13 and the experimental constructs in §11 are experimental.
A tool must not claim full v1 conformance until every mandatory static and
dynamic semantic has been promoted and implemented.

Syllog is a memory-safe, expression-oriented systems language with explicit
effects and domain constructs for native applications, streaming agent graphs,
audited optimization loops, and capability-restricted runtime evolution. AI
constructs do not weaken the ordinary type, ownership, effect, or capability
rules.

## 2. Source text and lexical structure

Source files use UTF-8 and the `.syl` extension. Invalid UTF-8 is a translation
error. Line endings are LF or CRLF. Outside literals and comments, whitespace
separates tokens but is otherwise insignificant.

Identifiers are Unicode XID-start followed by Unicode XID-continue, with `_`
permitted in either class. Implementations normalize identifiers to NFC before
comparison and diagnose distinct spellings that normalize identically in one
scope. A raw identifier, `r#match`, denotes the identifier `match`.

Line comments begin with `//`. Block comments use `/* ... */` and nest. Doc
comments are `///` and `/** ... */`. Strings are UTF-8 (`"text"`), byte strings
are `b"bytes"`, and raw strings use `r"..."` or `r#"..."#`. Character literals
contain one Unicode scalar. Integer bases are `0b`, `0o`, `0x`, or decimal;
underscores are separators. Floating literals follow IEEE-754 decimal or
hexadecimal notation. Duration and size suffixes such as `200ms`, `4GiB` are
typed literals, not textual macros.

Reserved keywords are:

```text
agent as asi_loop async await break capability component const continue
crate else enum evo extern false fn for if impl in let loop match mesh mod
module move mut on pipeline probe pub ref require return safety_bound self
state static stream struct trait true type unsafe use where while zk_verify
```

Contextual keywords include `activation`, `backpressure`, `budget`, `capture`,
`commit`, `determinism`, `emit`, `fallback`, `iteration`, `objective`, `provider`,
`reject`, `route`, `seed`, `target`, and `timeout`.

Operators and punctuation are:

```text
( ) [ ] { } , . : ; :: -> => @ # ?
+ - * / % ** & | ^ ! ~ = == != < <= > >= && || ?? .. ..= |> <-
```

## 3. Modules, declarations, and visibility

A source file starts with an optional `module` path. `use` imports names; glob
imports are forbidden in public modules. Declarations are private unless marked
`pub`; `pub(crate)` and `pub(in path)` restrict visibility. The global namespace
is split into types, values, modules, labels, and macros. Cyclic module values
are rejected unless every edge is a function or lazy static.

```syl
module acme.telemetry
use core.result.{Result, Error}

pub const MAX_BATCH: Usize = 256
pub fn flush(batch: &mut Batch) -> Result<(), Error> { batch.send() }
```

## 4. Core grammar

The following EBNF is normative at the syntactic level; semantic restrictions
in later sections still apply. `IDENT`, `LITERAL`, and balanced token trees are
lexical productions.

```ebnf
compilation-unit = [ "module" path ] , { use-decl | item } ;
item = [ visibility ] , ( function | struct | enum | type-alias | trait
     | impl | state | component | extern-block | agent | pipeline | mesh
     | probe | safety-bound | evo | asi-loop ) ;
visibility = "pub" , [ "(" , ( "crate" | "in" path ) , ")" ] ;
path = IDENT , { "::" , IDENT } ;
generic-params = "<" , generic-param , { "," , generic-param } , ">" ;
generic-param = IDENT , [ ":" , type , { "+" , type } ] ;
type = path [ "<" , type , { "," , type } , ">" ]
     | "&" [ lifetime ] [ "mut" ] type | "[" type ";" const-expr "]"
     | "(" [ type , { "," , type } ] ")" | "fn" function-signature
     | "impl" type | "dyn" type ;
function = [ "async" ] , [ "unsafe" ] , "fn" , IDENT , [ generic-params ]
         , "(" , [ parameter , { "," , parameter } ] , ")"
         , [ "->" , type ] , [ effect-set ] , block ;
parameter = pattern , ":" , type ;
block = "{" , { statement } , [ expression ] , "}" ;
statement = "let" pattern [ ":" type ] "=" expression [ ";" ]
          | expression [ ";" ] | item ;
expression = LITERAL | path | block | unary | binary | call | field | index
           | if-expr | match-expr | loop-expr | async-expr | await-expr
           | return-expr | break-expr | struct-expr | tuple-expr | array-expr ;
match-expr = "match" expression "{" { pattern [ "if" expression ]
           "=>" expression "," } "}" ;
```

Semicolons are optional after declarations and block-valued statements, and
required only to discard a non-`()` expression where a grammar ambiguity would
otherwise exist. Formatters emit semicolons only when necessary.

## 5. Type system

Syllog is statically typed with local Hindley–Milner-style inference constrained
by nominal public types, traits, ownership, effects, and lifetimes. Public
function parameters and results require annotations. There are no implicit
numeric conversions, truthiness, null, or unchecked downcasts.

Primitive types are `Bool`, `Char`, `I8`…`I128`, `U8`…`U128`, `Isize`, `Usize`,
`F16`, `F32`, `F64`, `Decimal128`, `String`, `Str`, `Bytes`, `Duration`, `Size`,
and `()`. Target-width integer use in serialized or FFI data is forbidden.

Product types use structs and tuples. Sum types use enums; every enum is a
tagged union, and `match` must be exhaustive. `Option<T>` is `none | some(T)`;
`Result<T,E>` is `ok(T) | err(E)`. The `?` operator returns the residual through
the enclosing function's `Try` implementation.

```syl
pub struct UserId(U128)

pub enum Lookup<T, E> {
    found(T),
    missing,
    failed(E),
}

pub fn name(result: Lookup<User, DbError>) -> Result<String, DbError> {
    match result {
        .found(user) => Ok(user.name),
        .missing => Ok("anonymous".into()),
        .failed(error) => Err(error),
    }
}
```

Generics are monomorphized by default. `dyn Trait` uses an explicit vtable.
Traits may declare associated types and constants. Coherence permits an impl
only when either the trait or self type is local. Specialization is absent.
Const generics are restricted to total, compile-time evaluable expressions.

Effects form an inferred set written `!{io, net, ai, time, random, unsafe}`.
Callers must possess every callee effect. Pure functions have the empty set.
Deterministic contexts reject `time`, `random`, unordered iteration, ambient
I/O, and nondeterministic accelerator kernels unless supplied through a seeded,
recordable capability.

## 6. Ownership, borrowing, and memory

Every value has one owner. Assignment, parameter passing, capture, and return
move non-`Copy` values. A value may be consumed at most once (affine semantics);
linear resource types marked `@must_use` must be consumed exactly once on every
path. `Drop` runs deterministically at the end of the owning scope. There is no
tracing garbage collector.

`&'a T` is a shared borrow and `&'a mut T` an exclusive borrow. During an
exclusive borrow no overlapping borrow may be used; during shared borrows the
referent may not be mutated. Lifetimes are inferred locally and written on
public APIs when elision is ambiguous. References cannot outlive referents.
Self-referential movable values require pinning.

`Send` permits ownership transfer across workers; `Sync` permits shared access.
Compiler-derived implementations require every field to satisfy the trait.
Interior mutation is available only through synchronized or single-worker cell
types. Data races are impossible in safe Syllog.

"Zero allocation" means no *implicit* heap allocation: stack values, borrowed
slices, arenas, fixed-capacity collections, and caller-provided buffers are
first-class. `String`, growable collections, boxed trait objects, and unbounded
stream buffering allocate explicitly and may fail with `AllocError`. Profiles
may set `#![deny(heap_alloc)]` or an allocation budget.

Isolation domains (`worker`, `agent`, `evo`, native UI main actor) exchange
owned `Send` messages, immutable shared regions, or capability handles. Raw
pointers exist only in `unsafe` blocks. Unsafe code must state and uphold its
safety contract; it cannot disable ownership checks outside the block.

### 6.1 Layout and ABI

`repr(syl)` layout is unspecified but stable within one compiler build.
`repr(C)` uses the target C ABI. `repr(transparent)` gives a single-field wrapper
the field ABI. `repr(wasm)` uses little-endian linear-memory records with fixed
alignment, 32-bit offsets on wasm32 and 64-bit offsets on wasm64. References
never cross an ABI boundary; `(offset, length, capacity)` descriptors do.

Enums use the smallest sufficient discriminant followed by aligned payload,
unless niche optimization eliminates the explicit tag. Niche layout is not an
external ABI promise. Native and Wasm serialization must use a versioned schema,
not raw `repr(syl)` bytes.

## 7. Concurrency and asynchronous execution

`async fn` returns `Task<T>`. `await` suspends without blocking a worker. Tasks
are lazy until awaited or spawned. Structured task groups cancel unfinished
children on scope exit. Detached tasks require a `Supervisor` capability.

Executors expose `.main`, `.io`, `.compute`, and `.ai` pools. The runtime uses
work stealing for movable tasks and actor affinity for UI/native objects.
Cancellation is cooperative at await points and explicit checks. Bounded
channels and streams require a policy: `block`, `drop_oldest`, `drop_newest`, or
`fail`. An unbounded channel is rejected in allocation-denied profiles.

## 8. Native targets, UI, and FFI

The compiler front end lowers typed Syllog IR to Cranelift for fast development,
LLVM for optimized ahead-of-time native artifacts, or canonical Wasm. Supported
triples include Apple arm64/x86_64, Android arm64/x86_64, Linux, Windows, and
macOS. Platform packs provide SDK metadata and native link steps. Runtime logic
does not require a JavaScript bridge.

`component` declares a retained state owner and a pure `view` that lowers to
SwiftUI/UIKit, Jetpack Compose/Views, AppKit, WinUI, or the desktop pack. Keys
give node identity. State mutation is confined to the main actor. Render diffing
is batched once per display tick; high-frequency event handlers may consume a
bounded lock-free event ring on a dedicated worker and publish coalesced state.

`extern "C"`, `extern "swift"`, and `extern "kotlin"` define ABI boundaries.
Only ABI-safe types cross directly. `Borrowed<T>` is valid for the dynamic call;
`Owned<T>` transfers destruction responsibility; `Shared<T>` uses an explicit
atomic retain/release adapter. `zero_copy` is accepted only when layout,
alignment, lifetime, thread affinity, and mutation exclusivity are proven.
Otherwise the compiler inserts a diagnosed copy or rejects the declaration.
Every foreign exception must map to a declared error value; unwinding may not
cross the boundary.

## 9. Agents

An `agent` is a typed, immutable route specification. Secrets are opaque
capabilities and cannot be interpolated, logged, serialized, or probed. Provider
adapters implement a versioned `ModelProvider` trait.

```ebnf
agent = "agent" IDENT "{" { agent-entry } "}" ;
agent-entry = "provider" ":" expression
            | "model" ":" STRING
            | "context_window" ":" INTEGER
            | "system" ":" STRING
            | "input" ":" type | "output" ":" type
            | "temperature" ":" FLOAT | "seed" ":" INTEGER
            | "timeout" ":" duration | "max_output_tokens" ":" INTEGER
            | "fallback" ":" array
            | "circuit_breaker" ":" record ;
```

The context window is a hard token upper bound. Before dispatch, adapters count
tokens using the selected model's pinned tokenizer; excess input yields
`ContextOverflow` unless a typed truncation/summarization policy exists.
`temperature: 0` is not a determinism guarantee. `determinism: strict` also
requires a pinned provider/model revision, seed support, canonical request
serialization, recorded tool results, and a provider reproducibility claim;
otherwise compilation or deployment validation fails.

Fallbacks are ordered. Retryable transport, overload, and configured 5xx errors
may advance; authentication, validation, safety refusal, and context overflow do
not unless explicitly mapped. Circuit breakers are per tenant and route, use a
monotonic clock, and expose closed/open/half-open states.

## 10. Pipelines and meshes

A `pipeline` is a typed structured-concurrency function with explicit capacity,
backpressure, cancellation, and ordering. `stream token in source { emit token }`
pulls one borrowed token frame at a time. Frames use slab-backed segments and
reference-counted immutable byte ranges; routing moves descriptors instead of
copying payloads. Retention beyond the next suspension point requires `freeze`
or ownership transfer.

A `mesh` declares a deployment graph. `route A -> B partition by key parallelism
N` preserves order within a partition and permits cross-partition reordering.
Cycles require a delay/buffer node and a bounded capacity. Supervision policies
define restart limits; exactly-once effects require transactional sinks and
checkpointed offsets. The type checker verifies edge type compatibility.

Context caches are tenant-scoped, encrypted at rest, bounded by bytes and entry
count, and keyed by provider/model/tokenizer plus prompt schema. Secret-bearing
or user-specific entries may not enter a shared cache. Eviction is observable
but cannot change semantic correctness.

## 11. Probes and safety bounds

`probe P on target { ... }` attaches a read-only observer to an instrumentable
local model or provider extension. Captured tensors carry element type, rank,
shape variables, device, and lifetime. Probe callbacks cannot mutate model
state. Sampling, retention, redaction, time budget, and failure policy are
mandatory in production profiles. Probe data is sensitive and capability
protected.

Closed remote model APIs normally do not expose attention tensors or hidden
activations. In that case activation probes are statically unavailable; Syllog
does not infer private activations or claim to verify hidden chain-of-thought.
Textual rationale checks are ordinary output evaluations, not mechanistic
interpretability.

A `safety_bound` is a total, side-effect-free predicate plus optional proof
verification. Every `require` must evaluate true before the guarded operation.
Failure returns `BoundViolation` and cannot be caught inside the guarded `evo`
candidate. Bounds have stable identifiers and hashes included in audit events.

`zk_verify Scheme { verification_key, proof, public_inputs }` verifies a proof
against a pinned circuit and key. It proves only the circuit statement over the
declared public inputs; it does not by itself prove alignment, harmlessness, or
semantic safety. Key provenance, circuit hash, and trusted setup metadata are
deployment artifacts. Verification must be constant-time where the scheme
requires it.

## 12. Evolution modules and hot swapping

An `evo` declaration defines a typed Wasm component slot, never unrestricted
self-modifying native code. Generated source passes the normal parser, type and
effect checker, capability linker, Wasm validator, fuel/memory limiter, safety
bounds, proof gates, evaluation suite, and health check.

Candidates execute in a fresh store with no ambient authority. Imports are
capability allowlists. Linear memory, tables, stack, fuel, epoch deadline, output
bytes, and host-call rate are bounded. WASI is absent unless an explicit,
virtualized capability is granted. Native host pointers cannot enter the store.

Promotion is generation-based: compile and instantiate off-path; migrate state
through a versioned, transactional function; run health checks; atomically
publish the new generation; drain calls on the old generation; retain it for
rollback. In-flight calls finish on their captured generation. "Zero downtime"
means no planned rejection at promotion; it does not promise zero latency or
survival of host failure. A failed migration or health window leaves or restores
the prior generation.

## 13. Autonomous optimization loops

`asi_loop` is a supervised bounded search construct, not an exemption from host
control. It requires an objective, deterministic or recorded evaluation method,
budgets, stopping rule, candidate isolation, safety bound, audit sink, and human
or policy authorization capability for promotion.

Metrics are typed, versioned, and direction-aware. Candidate comparison uses the
same dataset revision, seed set, hardware class, and numeric mode. Held-out
suites cannot be read by the candidate. Multiple objectives require an explicit
Pareto or scalarization rule. A loop cannot modify its own safety bounds,
verification keys, evaluator, budget, audit log, or promotion capability.

Every iteration records input artifact hashes, compiler version, random seed,
candidate hash, proof result, probe summary, metrics with confidence intervals,
decision, active generation, and rollback state. Audit records are hash chained
and may be externally anchored. Host operators may pause, revoke capabilities,
or roll back at any time.

## 14. Implemented front-end grammar

The current Pest front end accepts the following implemented subset and attaches
an exact byte/line/column span to every structural AST node:

```ebnf
program = { struct-decl | enum-decl | function-decl | state-decl
        | agent-decl | pipeline-decl | safety-bound-decl } ;
struct-decl = [ "pub" ] , "struct" , identifier , "{" , { struct-field } , "}" ;
enum-decl = [ "pub" ] , "enum" , identifier , "{" , { enum-variant } , "}" ;
function-decl = [ "pub" ] , [ "async" ] , "fn" , identifier
              , parameters , [ "->" , type ] , block ;
state-decl = [ "pub" ] , "state" , identifier , "{" , { state-field } , "}" ;
agent-decl = "agent" , identifier , block ;
pipeline-decl = "pipeline" , identifier , [ parameters ]
              , [ "->" , type ] , property-block ;
safety-bound-decl = "safety_bound" , identifier , [ parameters ] , property-block ;
property = identifier , ":" , ( expression | type , "=" , expression ) ;
expression = literal | path | array | call | field-access | prefix | infix
           | block | match-expression ;
```

It produces a typed-syntax `Ast` with declarations, types, statements,
expressions, patterns, match arms, call arguments, and source spans. "Typed"
here means annotations are represented structurally. The current semantic pass
resolves names and implemented algebraic types, checks expression and pipeline
compatibility, and checks supported closed matches for exhaustiveness. The
complete v1 examples still contain declarations
such as `component`, `impl`, `mesh`, `probe`, `evo`, and `asi_loop` outside this
milestone and therefore remain future conformance fixtures.

### 14.1 Executable normative rules

Every rule below has at least one accepted and one rejected program in
`spec/cases/manifest.json`. A change to an identifier or its observable
diagnostic behavior follows the compatibility policy in §15.

| Rule identifier | Current normative requirement |
| --- | --- |
| `SYL-SYNTAX-ITEM-001` | A compilation unit contains only grammar-supported declarations; malformed items emit `SYL0001`. |
| `SYL-SYMBOL-UNIQUE-001` | Type and value declarations are unique within their namespace; duplicates emit `SYL2001`. |
| `SYL-TYPE-NAME-001` | Every referenced type resolves to a primitive or declared type; unknown types emit `SYL2002`. |
| `SYL-VALUE-NAME-001` | Every referenced value, constructor, variant, and field resolves; unknown names emit `SYL2003`. |
| `SYL-TYPE-OPTION-001` | `Option<T>` takes exactly one type argument and supports `some(T)` and `none`. |
| `SYL-TYPE-RESULT-001` | `Result<T,E>` takes exactly two type arguments and supports `ok(T)` and `err(E)`. |
| `SYL-TYPE-COMPAT-001` | Checked initializers, calls, and returns must be type-compatible; mismatches emit `SYL2101`. |
| `SYL-MATCH-EXHAUSTIVE-001` | Matches over `Bool`, enums, `Option`, and `Result` must cover every remaining case; failures emit `SYL2301`. |
| `SYL-DOMAIN-UNIQUE-001` | Agent, pipeline, and safety-bound property names are unique; duplicates emit `SYL1001`. |
| `SYL-AGENT-REQUIRED-001` | An agent defines `provider` and `context_window`; missing properties emit `SYL1002`. |
| `SYL-AGENT-PROVIDER-001` | A provider is a non-empty route string with model or a valid named-argument provider call; malformed definitions emit `SYL1201`. |
| `SYL-AGENT-FALLBACK-001` | Fallback is an array of self-contained non-empty routes or provider calls; malformed entries emit `SYL1202`. |
| `SYL-PIPELINE-REFERENCE-001` | A pipeline's required `agent` property names a declared agent; invalid references emit `SYL1101`. |
| `SYL-PIPELINE-CONTRACT-001` | A typed pipeline input and output are compatible with the selected typed agent; failures emit `SYL2201`. |
| `SYL-SAFETY-REQUIRED-001` | A safety bound contains at least one `require` or `policy` property; omission emits `SYL1002`. |

## 15. Diagnostics, profiles, and security

Diagnostics carry byte span, Unicode line/column, stable code, explanation, and
machine-applicable fixes where unambiguous. Parsing recovers at item boundaries;
type checking continues with error types without emitting executable artifacts.
The implemented configuration diagnostic codes and validation rules are defined
in [`docs/diagnostics.md`](diagnostics.md).
Implemented name resolution, algebraic typing, pipeline compatibility, and
match-exhaustiveness semantics are detailed in
[`docs/semantic-analysis.md`](semantic-analysis.md).

Profiles are `dev`, `release`, `deterministic`, `mobile`, `wasm-sandbox`, and
`safety-critical`. Safety-critical forbids `unsafe`, ambient capabilities,
unbounded allocation, unwinding, dynamic linking, unpinned providers, and
unverified evolution. Reproducible builds pin compiler, target pack, provider
schema, tokenizer, and dependencies and emit an SBOM plus provenance statement.

The compiler treats prompts, model output, foreign input, generated code, Wasm,
proofs, and probe frames as untrusted. Taint analysis prevents untrusted strings
from becoming capabilities, module paths, SQL, shell commands, or native symbols
without typed validation. Logs redact secrets and tenant data by type. Package
signatures establish provenance, not trust; policy still constrains capabilities.

## 16. End-to-end examples

The normative, complete programs are maintained as executable-design fixtures:

- `examples/native_mobile.syl`: reactive state, native UI tree, async HTTP, and
  ownership-explicit Swift/Kotlin bridges.
- `examples/enterprise_agents.syl`: typed agent routes, fallbacks, circuit
  breaking, bounded streaming, context caching, and a supervised mesh.
- `examples/autonomous_evo.syl`: activation probes, proof-gated safety bounds,
  bounded Wasm evolution, atomic generations, and audited optimization.

They contain concrete types, budgets, policies, endpoints, error paths, and
lifecycle behavior; no ellipses or placeholder bodies are used.
