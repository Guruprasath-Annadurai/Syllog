# Supported Platforms

## Status

Syllog has bootstrap-level host support, not a stable platform guarantee.

## Compiler hosts

The required CI design builds and tests the full workspace on:

| Host | Tier | Current promise |
| --- | --- | --- |
| Ubuntu latest x86-64 | Bootstrap CI | Required to pass before merge |
| macOS latest Apple Silicon or x86-64 runner | Bootstrap CI | Required to pass before merge |
| Windows latest x86-64 | Bootstrap CI | Required to pass before merge; shell scripts run through Git Bash |

Only the exact Rust toolchain in `rust-toolchain.toml` is supported. A green CI
run is evidence for that commit, not a long-term compatibility guarantee.

## Execution targets

`wasm32-syllog` is the only Syllog code-generation target in the workspace. It
emits a limited Wasm core program and executes through Wasmtime. Syllog does not
currently ship LLVM/Cranelift native code generation, iOS/Android targets,
native UI bindings, a stable C/Swift/Kotlin FFI, or release binaries.

## Compatibility

Platform support may change during `0.x`. Removing a CI host or changing a
public target name requires an RFC, migration notes, and a version-policy review.
Platform-specific failures must not be silently ignored.
