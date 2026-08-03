# Syllog

Syllog (`.syl`) is an experimental systems language for native applications,
bounded agent orchestration, and capability-restricted cognitive optimization.
This repository contains its Rust bootstrap workspace and draft v1 language
reference.

> The implementation is a compiler front end: parsing, domain validation, name
> resolution, and static type checking are implemented. Ownership checking,
> lowering, code generation, execution, and advanced v1 constructs remain future
> milestones.

## Workspace

```text
crates/syllog-cli      `syllog check` and `syllog run` entry point
crates/syllog-compiler parse/resolve/type-check orchestration and presentation
crates/syllog-parser   Pest grammar, AST, and `parse_syl`
crates/syllog-semantic symbol tables, type resolution, and static checks
crates/syllog-proxy    asynchronous model route/circuit-breaker primitives
crates/syllog-runtime  policy-enforced Wasmtime execution foundation
examples/              bootstrap and v1 conformance-design programs
docs/                  language reference, design notes, and grammar
```

The pre-workspace hand-written lexer/interpreter remains in `src/` as preserved
historical work; it is not currently a workspace member.

## Build and test

Rust 1.85 or newer is required.

```bash
cargo build --workspace
cargo test --workspace
cargo run -p syllog-cli -- check examples/hello_agent.syl
cargo run -p syllog-cli -- check examples/core_frontend.syl
cargo run -p syllog-cli -- check examples/semantic_frontend.syl
cargo run -p syllog-cli -- check examples/semantic_frontend.syl --json
cargo run -p syllog-cli -- run examples/hello_agent.syl
```

At this milestone `run` validates the program and initializes the command path;
execution semantics will be connected in a later runtime milestone.

## Implemented syntax

```syl
agent assistant {
    provider: "openai"
    model: "gpt-5"
    context_window: 128000
    deterministic: true
}

pipeline answer_request {
    input: text
    agent: assistant
    output: stream
}

safety_bound output_policy {
    policy: "Never emit secrets or personal data"
    enforced: true
}
```

The parser also accepts span-aware declarations for `struct`, `enum`, `state`,
`fn`/`async fn`, typed function and pipeline signatures, typed domain properties,
arrays, calls with positional or named arguments, field access, prefix/infix
expressions, blocks, local bindings, returns, and `match`. See
[`examples/core_frontend.syl`](examples/core_frontend.syl).

`syllog check` is the front-end compilation command. It parses the complete
file, validates domain declarations, resolves type and value names, and then
type-checks expressions, functions, agent/pipeline contracts, and supported
closed matches. It emits contextual terminal diagnostics by default. Pass
`--json` (or `--diagnostic-format=json`) for a versioned editor report on stdout;
compile errors still produce a failing process status. See
[the diagnostics reference](docs/diagnostics.md) for the format and stable codes.

The semantic pass now provides separate type/value symbol tables, forward name
resolution, primitive and algebraic types, `Option`/`Result`, typed call and
return checking, agent/pipeline contract compatibility, and exhaustiveness for
closed enums, `Bool`, `Option`, and `Result`. Its precise scope is documented in
[the semantic analysis reference](docs/semantic-analysis.md).

The runtime foundation now includes an object-safe provider adapter, a
deterministic mock provider, bounded Tokio token streaming with backpressure,
and fresh-store Wasmtime execution with fuel, per-memory, and deny-by-default
host capability enforcement. Its supported ABI and deliberately deferred
`evo`/`asi_loop` work are documented in
[the runtime foundations reference](docs/runtime-foundations.md).

See [the Language Reference Manual](docs/language-reference.md) for the v1 type,
ownership, native, agent, probe, safety, Wasm evolution, and `asi_loop` design.

## License

Licensed under Apache License 2.0. See [LICENSE](LICENSE).
