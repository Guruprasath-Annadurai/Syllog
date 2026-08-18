# Contributing to Syllog

Syllog is developed as a compiler and runtime safety project. Changes must be
small enough to review, test-first, deterministic, and explicit about language
compatibility and host capabilities.

## Required tools

- Rust `1.86.0` with `rustfmt` and `clippy`. Wasmtime 36 and its Cranelift
  dependencies establish this minimum supported Rust version.
- `cargo-deny 0.20.2`. It requires Rust 1.88 to build, but audits the Rust
  1.86-compatible Syllog workspace after installation. Install it with
  `cargo +1.88.0 install --locked cargo-deny --version 0.20.2`.
- Git and a Linux, macOS, or Windows host. Windows uses Git Bash for the
  repository gate; see `docs/supported-platforms.md` for the bootstrap support
  level.

The repository's `rust-toolchain.toml` is authoritative. Developers using
rustup receive the pinned toolchain automatically.

## Development workflow

Work on a feature branch or isolated worktree. Preserve unrelated working-tree
changes and never combine them into a Syllog commit.

For behavior changes:

1. Add one focused test that describes the user-visible or compiler-visible
   contract.
2. Run it and confirm it fails because the behavior is absent.
3. Implement the smallest complete change that satisfies the contract.
4. Run the focused test, then `bash scripts/ci.sh`.
5. Commit the test, implementation, and directly related documentation
   together.

## Mandatory local gate

```bash
bash scripts/ci.sh
```

The script verifies formatting, strict Clippy lints, all targets with all
features, workspace tests/builds, dependency advisories/licenses/bans/sources,
and checked-in Syllog programs in human and JSON diagnostic modes. Pull requests
must pass the same gate on Linux, macOS, and Windows.

## Compatibility expectations

- Parser or semantic changes require positive and negative conformance cases.
- Public diagnostic codes and JSON schemas cannot change silently.
- Runtime imports are denied unless represented by an explicit capability.
- New unbounded allocations, queues, tasks, or retries require a documented
  policy owner and limit.
- Experimental `evo` and `asi_loop` work is excluded from stable builds until
  the post-1.0 research gates are satisfied.

Security vulnerabilities should not be opened as public issues. Until a
dedicated security contact is published, report them privately to the repository
owner through GitHub's private vulnerability reporting interface.
