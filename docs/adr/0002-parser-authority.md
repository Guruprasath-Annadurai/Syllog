# ADR 0002: Pest Grammar and Workspace Compiler Are Authoritative

## Status

Accepted for the bootstrap architecture. Owner: Compiler Working Group. Revisit
only through an RFC/ADR pair if parser technology or compiler boundaries change.

## Decision

`crates/syllog-parser/src/grammar.pest` is the single executable grammar and
`crates/syllog-parser` is the single parser/AST implementation. The EBNF document
is explanatory and cannot override parser behavior. `crates/syllog-compiler` is
the single phase orchestrator; downstream execution follows ADR 0001.

The root `src/` hand-written prototype is frozen, excluded from the virtual Cargo
workspace, and retained only as history. Alternatives considered were deleting
it immediately, maintaining two parsers, or restoring it as authoritative.
Deletion would erase context without explicit approval; dual maintenance creates
semantic drift; restoring it would discard the tested Pest/HIR/MIR pipeline.

Consequences: grammar changes require parser/conformance tests and EBNF updates;
compiler phase changes require diagnostics and architecture updates. The reversal
strategy is a separately accepted migration ADR with differential corpus evidence
before authority changes.

## Compatibility

This decision changes no accepted `.syl` source. It clarifies authority and
prevents historical code from creating an implied compatibility promise. A
future parser replacement must preserve the accepted corpus or use edition and
migration policy.

## Security impact

One parser reduces inconsistent validation of untrusted source. It does not make
the parser safe by declaration; malformed-input regression tests, fuzzing, and
resource bounds remain required. Historical code must not be shipped accidentally
as an alternate parsing path.
