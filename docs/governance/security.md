# Syllog Security Policy

## Status

Active bootstrap policy. Owner: Runtime and Security Working Group. Review
cadence: quarterly, after a critical incident, and before every stable release.

## Decision

Security reports use GitHub private vulnerability reporting for this repository.
Public issues must not contain undisclosed exploit details. The owner acknowledges
a private report within three business days, assigns severity within seven days,
and coordinates remediation and disclosure with the reporter.

Sandbox imports and deployment capabilities are denied by default. Build and
runtime inputs are treated as untrusted. Secrets are represented by opaque
capability handles and are excluded from diagnostics, traces, caches, artifacts,
and test snapshots. No stable release artifacts exist today. Checksums, signed
provenance, SBOMs, and a reproducible release workflow are mandatory before a
production-readiness claim.

## Compatibility

Security releases preserve source compatibility when safe. If continued
compatibility would preserve an exploitable path, the release disables it with a
stable diagnostic, security advisory, affected-version range, and migration
instructions. Revoked artifact signatures or capabilities fail closed.

## Security impact

This policy creates disclosure and response expectations but is not itself a
sandbox. Enforcement resides in compiler effects, artifact verification, runtime
fuel/memory limits, and capability linkers. Current limitations are documented in
`docs/runtime-foundations.md`; operators must not treat unimplemented controls as
present.
