# Syllog

[![CI](https://github.com/Guruprasath-Annadurai/Syllog/actions/workflows/ci.yml/badge.svg)](https://github.com/Guruprasath-Annadurai/Syllog/actions/workflows/ci.yml)

Syllog is an experimental programming-language implementation for bounded,
capability-aware AI components. This repository is a Rust bootstrap compiler,
Wasm runtime, package prototype, and conformance suite. It is not production,
enterprise, or Rust-level memory-safety ready.

## What works today

The implemented path parses `.syl` source with Pest, performs domain validation,
name resolution, type checks, limited ownership/effect analysis, lowers an
executable integer/Boolean/unit subset through HIR and verified MIR, emits Wasm,
and runs that subset in a fuel- and memory-limited Wasmtime store. Package-aware
`build` and `run`, deterministic lockfiles, offline cache/vendor inputs, provider
adapter prototypes, and a development rebuild loop also exist with the limits in
[the feature-status matrix](docs/feature-status.md).

The following are not complete: Rust-equivalent ownership and borrowing, async
source-language execution, end-to-end agent/pipeline execution from `.syl`, a
hosted package registry, a usable standard library, native backends/UI, signed
release provenance, and `evo`/`asi_loop`.

## Architecture

```text
.syl source
  -> Pest parser + span-aware AST
  -> domain validation + name/type resolution
  -> limited ownership + effect checks
  -> typed HIR
  -> MIR + mandatory verifier
  -> reference interpreter or Wasm backend
  -> policy-configured Wasmtime runtime
```

The authoritative grammar is
[`crates/syllog-parser/src/grammar.pest`](crates/syllog-parser/src/grammar.pest).
The authoritative pipeline is in
[`crates/syllog-compiler`](crates/syllog-compiler). See
[the architecture guide](docs/architecture.md) for crate responsibilities and
trust boundaries. The old root `src/` implementation is preserved historical
code and is not a Cargo workspace member.

## Build and test

Install `rustup`, Git, and `cargo-deny 0.20.2`. The repository pins Rust 1.86.0
with `rustfmt` and `clippy` in `rust-toolchain.toml`.

```bash
git clone https://github.com/Guruprasath-Annadurai/Syllog.git
cd Syllog
rustup show
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 install --locked cargo-deny --version 0.20.2
bash scripts/ci.sh
```

The separate Rust 1.88.0 toolchain is needed only to install the pinned
`cargo-deny`; repository code is built with Rust 1.86.0. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and
[supported platforms](docs/supported-platforms.md) for current coverage.

## Try the implemented subset

```bash
cargo run -p syllog-cli -- check examples/hello_agent.syl
cargo run -p syllog-cli -- check examples/semantic_frontend.syl --json
cargo run -p syllog-cli -- build spec/cases/runtime/exit_42.syl \
  --target wasm32-syllog --output /tmp/exit_42.wasm
cargo run -p syllog-cli -- run spec/cases/runtime/exit_42.syl \
  --fuel 100000 --memory-bytes 65536
```

`hello_agent.syl`, `core_frontend.syl`, and `semantic_frontend.syl` are checked
examples. The native, enterprise-agent, and autonomous-evolution files under
`examples/` are non-executable design fixtures; they intentionally use syntax
and libraries the compiler does not support.

## Project policy

- [Feature status](docs/feature-status.md)
- [Design](docs/design.md)
- [Language reference and design material](docs/language-reference.md)
- [Security policy](SECURITY.md)
- [Roadmap](ROADMAP.md)
- [RFC process](docs/governance/rfc-process.md)
- [ADR process](docs/governance/adr-process.md)

## License

Licensed under Apache License 2.0. See [LICENSE](LICENSE).
