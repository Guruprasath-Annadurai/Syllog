# Syllog Architecture Decision Record Process

## Status

Active bootstrap process. Owner: Compiler Working Group. Review cadence:
semiannually and whenever the compiler/runtime boundary changes.

## Decision

Significant architectural decisions are recorded as sequential Markdown files
under `docs/adr/`. An ADR must state context, decision, alternatives considered,
consequences, compatibility, security impact, and reversal strategy. Status is
one of Proposed, Accepted, Superseded, or Rejected. A superseding ADR links both
directions and does not rewrite historical decisions.

An ADR records how an approved design is structured. An RFC is additionally
required for new syntax, effects/capabilities, public formats, trust boundaries,
registry protocols, ownership rules, runtime ABIs, and evolution/promotion
mechanisms. Code cannot cite an ADR as evidence that behavior is implemented.

## Compatibility

ADRs identify affected source editions, APIs, manifests, lockfiles, artifacts,
and migration requirements. Internal refactors with no public impact still state
that conclusion explicitly. Accepted decisions remain reviewable; reversal uses
a new ADR and preserves any required compatibility window.

## Security impact

Every ADR identifies changed trust boundaries, untrusted inputs, authority,
resource limits, failure behavior, and rollback. A missing security analysis
blocks acceptance for compiler execution, packages, providers, runtime hosting,
secrets, telemetry, or dynamic loading.
