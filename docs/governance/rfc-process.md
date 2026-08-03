# Syllog RFC Process

## Status

Active bootstrap process. Owner: Language Steering Group. Review cadence:
semiannually and after each completed language edition.

## Decision

Changes to syntax, static semantics, runtime ABI, capabilities, package protocol,
artifact format, or compatibility policy require a numbered RFC. An RFC moves
through `Draft`, `Review`, `Accepted`, `Implemented`, or `Rejected`. Review lasts
at least 14 calendar days unless the documented security emergency process
applies.

Every RFC states motivation, precise grammar or interfaces, alternatives,
conformance tests, compatibility, security impact, rollout, and rollback. The
Language Steering Group owns language decisions; Runtime and Security owners
must approve new host effects or capabilities. Acceptance does not make a feature
stable: implementation, conformance evidence, and release notes are required.

## Compatibility

An RFC identifies affected editions and protocol versions. Accepted breaking
source changes target a new edition. Protocol changes use explicit version
negotiation. Implementations cannot repurpose an existing diagnostic code,
artifact version, capability name, or manifest field with different semantics.

## Security impact

RFCs introducing code execution, external communication, secrets, persistence,
dynamic loading, or model access require threat modeling and Security owner
approval. Missing security analysis blocks acceptance. `evo` and `asi_loop` RFCs
remain in the experimental channel until all Phase 12 gates are independently
verified.
