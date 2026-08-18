# Syllog Design

**Status:** Bootstrap implementation; language version 0.1.0, edition 2026.

## Product boundary

Syllog is exploring whether a statically inspectable language and a
deny-by-default component runtime can improve the construction of bounded AI
systems. The current repository proves parts of a compiler and runtime path. It
does not prove enterprise readiness, native portability, full memory safety,
deterministic external-model behavior, or safe autonomous improvement.

## Sources of authority

There is one active parser. `crates/syllog-parser/src/grammar.pest` is the
authoritative grammar consumed by Pest, and `crates/syllog-parser` owns AST
construction and source spans. `docs/grammar.ebnf` is an explanatory projection;
if it disagrees with the Pest grammar, the Pest grammar and conformance tests win.

There is one active compiler pipeline. `crates/syllog-compiler` orchestrates
parsing, domain validation, resolution, type checking, ownership/effect checks,
HIR/MIR lowering, and diagnostic presentation through dedicated workspace
crates. Backends may consume only MIR accepted by `syllog-ir`'s verifier. The
reference interpreter defines behavior for the executable subset and is compared
with the Wasm backend by conformance tests.

The pre-workspace hand-written lexer/parser under root `src/` is historical and
not a Cargo workspace member. It is retained for provenance only and must not be
extended or cited as the current implementation.

## Current compiler path

```text
source bytes
  -> Pest grammar and span-aware AST             syllog-parser
  -> agent/pipeline/safety declaration checks    syllog-compiler
  -> symbols, names, and types                    syllog-semantic
  -> limited affine/borrow/effect checks          syllog-semantic/compiler
  -> typed HIR                                    syllog-compiler
  -> control-flow MIR and verifier                syllog-ir
  -> reference execution                          syllog-interpreter
     or deterministic Wasm emission               syllog-codegen-wasm
  -> bounded component execution                  syllog-runtime
```

Failures at trust boundaries are diagnostics or typed errors. A parser failure
currently stops AST production; recovery to a partial AST is not implemented.

## Design priorities

1. Make one small language subset coherent from source through both execution
   paths.
2. Keep diagnostics stable, span-aware, and machine-readable.
3. Deny undeclared host capabilities and bound runtime resources.
4. Version public source, diagnostic, manifest, lockfile, and artifact formats.
5. Prefer conformance and adversarial tests over readiness claims.

## Compatibility and security

Language and format changes follow
[`docs/governance/versioning.md`](governance/versioning.md). New syntax, effects,
capabilities, trust boundaries, registry protocols, ownership rules, or runtime
ABIs require an RFC. Architectural changes require an ADR. Runtime controls are
documented in [`docs/runtime-foundations.md`](runtime-foundations.md); controls
listed as missing there must not be inferred from aspirational language syntax.

## Explicit non-goals for the current release

- Native mobile UI or native machine-code generation.
- Access to hidden reasoning or activations of closed model APIs.
- AGI, ASI, consciousness, guaranteed correctness, or self-promotion.
- A stable public registry or production secret-management service.
- Rust-equivalent borrow checking or memory-safety claims.

The detailed implemented/partial/planned split is maintained in
[`docs/feature-status.md`](feature-status.md).
