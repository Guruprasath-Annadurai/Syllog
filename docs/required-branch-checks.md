# Required Branch Checks

Protect `main` after the updated workflow has run and GitHub has registered the
contexts. Require pull requests, at least one approving review when another
maintainer is available, dismissal of stale approvals, resolution of
conversations, and these exact checks:

- `workspace (ubuntu-latest)`
- `workspace (macos-latest)`
- `workspace (windows-latest)`

Do not require the credential-bearing scheduled `live` provider job for ordinary
pull requests; it is an opt-in smoke signal, not a deterministic merge gate.
Administrators should not bypass required checks except for a documented
security emergency. Branch deletion, force pushes, and history rewrites should
remain disabled.

The repository cannot claim this protection is active until GitHub settings are
queried and recorded in a layer report.
