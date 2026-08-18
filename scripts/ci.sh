#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repository_root"

if [[ ! -f deny.toml ]]; then
  echo "error: deny.toml is required for dependency policy enforcement" >&2
  exit 1
fi

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "error: cargo-deny is required; install the version pinned by CI" >&2
  exit 1
fi

required_cargo_deny_version="0.20.2"
installed_cargo_deny_version="$(cargo deny --version | awk '{print $2}')"
if [[ "$installed_cargo_deny_version" != "$required_cargo_deny_version" ]]; then
  echo "error: cargo-deny $required_cargo_deny_version is required; found $installed_cargo_deny_version" >&2
  exit 1
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --workspace --all-targets --all-features
cargo deny check
cargo run --quiet -p syllog-cli -- check examples/hello_agent.syl
cargo run --quiet -p syllog-cli -- check examples/semantic_frontend.syl --json
