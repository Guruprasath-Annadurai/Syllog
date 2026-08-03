# ADR 0001: Explicit Compiler Intermediate Representations

## Status

Accepted for staged implementation. Owner: Compiler Working Group. Review window:
2026-08-03 through 2026-08-17; revisit after the first differential Wasm backend
suite is operational.

## Decision

Syllog compilation uses explicit boundaries:

```text
source -> span-aware AST -> resolved typed HIR -> verified MIR -> backend artifact
```

The AST preserves source syntax and diagnostics. HIR replaces name strings with
stable definition identities and assigns every executable expression a resolved
type. MIR makes control flow, temporaries, calls, drops, suspension, and aggregate
layout explicit. Every backend consumes only MIR that passes the internal
verifier. A deterministic MIR interpreter defines the executable reference used
for differential backend testing.

## Compatibility

AST, HIR, and MIR are compiler-internal until separately versioned. Editor tools
consume compiler queries and the public diagnostic schema rather than serialized
internal representations. Artifact metadata records its format, compiler, target,
source hash, entry point, and required runtime capabilities.

## Security impact

The verified MIR boundary prevents malformed internal control flow or types from
reaching Wasm or native emitters. It does not prove source-level ownership or
capability safety by itself; those analyses must complete before MIR verification.
Backends cannot add undeclared imports or broaden artifact capabilities.
