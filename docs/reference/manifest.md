# Syllog project manifest

`Syllog.toml` is the strict, versioned project boundary used by Syllog tools.
Unknown keys are errors so misspelled security and build settings never degrade
silently.

```toml
[package]
name = "frontier-agent"
version = "0.1.0"
edition = "2026"

[[targets]]
name = "agent"
kind = "bin"
path = "src/main.syl"

[dependencies]
telemetry = "1.2.3"

[capabilities]
profile = "agent"
network = ["api.example.com:443"]
environment = ["MODEL_API_KEY"]
max_memory_bytes = 67108864
```

Target paths are relative to the manifest directory, normalized lexically, and
must remain within the project root. Target names must be unique. Capability
profiles are `none`, `agent`, or `native`; every concrete authority is still
listed explicitly.

Editors can obtain the versioned JSON Schema without filesystem discovery:

```console
syllog schema manifest
```

Manifest diagnostics are JSON-serializable and contain a stable code, filename,
and one-based line/column range.
