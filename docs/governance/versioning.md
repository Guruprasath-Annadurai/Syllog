# Syllog Versioning and Compatibility Policy

## Status

Active bootstrap policy. Owner: Compiler Working Group. Review cadence:
quarterly and before every minor or major release.

## Decision

The compiler, runtime, CLI, package protocol, artifact format, and editor JSON
schema use semantic versions independently. Syllog source declares a language
edition; the bootstrap edition is `2026`. Patch releases cannot change accepted
program behavior. Minor `0.x` releases may add syntax or diagnostics but must
provide migration notes. Beginning with `1.0`, source-breaking changes require a
new edition and an automated migration where syntax permits one.

Normative language rules receive stable identifiers. Diagnostic codes remain
stable within an edition. Serialized protocols and artifacts begin with an
integer schema or format version and reject unsupported versions explicitly.

## Compatibility

The current `0.x` line makes no general binary compatibility promise, but each
release publishes its supported source editions, target triples, runtime ABI,
JSON schema, and artifact versions. A compiler must either consume a declared
version correctly or emit a version-specific error; silent fallback is forbidden.

After `1.0`, the two newest language editions and the previous minor artifact
format are supported. Security fixes may disable behavior only through the
documented emergency process and must include a diagnostic and migration path.

## Security impact

Version negotiation is never authorization. Older artifacts retain no ambient
capabilities, and loading an older supported format cannot bypass current
signature, provenance, fuel, memory, or capability policy. Unsupported formats
fail closed before instantiation.
