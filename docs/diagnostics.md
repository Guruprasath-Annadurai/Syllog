# Syllog Compiler Diagnostics

The `syllog-compiler` crate exposes `compile(filename, source) -> Compilation`.
It runs the front end in this order:

```text
parse -> validate -> resolve -> type_check -> ownership -> effect_check
```

A syntax failure stops compilation after `parse`. Once parsing succeeds, domain
validation and semantic phases accumulate selected independent diagnostics. The
ownership phase runs after type checking; effect checking runs only when HIR
lowering is eligible. The internal compilation result retains the AST and symbol
table for later stages. `completed_phases` means a phase was attempted, not that
it emitted no errors, and a failed prerequisite can prevent a later phase.

The default terminal renderer prints the stable code, message, source location,
relevant source line, primary underline, and originating phase:

```text
error[SYL1201]: provider must be a non-empty string or provider call
 --> path/file.syl:2:5
2 |     provider: 42
  |     ^^^^^^^^^^^^
 = phase: validate
```

`syllog check FILE --json` and
`syllog check FILE --diagnostic-format=json` write exactly one JSON object to
stdout. Compiler diagnostics do not write to stderr in this mode. A report with
errors still exits unsuccessfully, making the mode useful to both editors and CI.
File I/O and invalid command-line usage are operational failures and remain
human-readable stderr errors.

## Editor JSON schema v1

```json
{
  "schema_version": 1,
  "success": false,
  "completed_phases": ["parse", "validate", "resolve", "type_check", "ownership", "effect_check"],
  "diagnostics": [
    {
      "code": "SYL2003",
      "severity": "error",
      "message": "unknown value 'missing'",
      "file": "src/main.syl",
      "phase": "resolve",
      "range": {
        "start": { "line": 0, "column": 31, "byte": 31 },
        "end": { "line": 0, "column": 38, "byte": 38 }
      }
    }
  ]
}
```

Ranges are half-open. JSON lines, columns, and byte offsets are zero-based;
columns count UTF-8 source characters as reported by the current parser. This is
not an LSP UTF-16 position, so an LSP adapter must convert columns for non-ASCII
source. The human-facing diagnostic model uses one-based lines and columns.

The schema intentionally excludes AST and symbol-table internals. Consumers must
branch on `schema_version` before assuming fields added by a future schema.

## Implemented codes

| Code | Meaning |
| --- | --- |
| `SYL0001` | Pest syntax error; no AST is produced. |
| `SYL0002` | Internal AST lowering failure after a successful parse. |
| `SYL1001` | Duplicate property in an `agent`, `pipeline`, or `safety_bound`. |
| `SYL1002` | Required domain property is missing. |
| `SYL1101` | Pipeline agent reference is malformed or does not name a declared agent. |
| `SYL1201` | Provider or model definition is malformed. |
| `SYL1202` | Fallback is not an array or contains a malformed provider definition. |

## Validation contract

An agent requires `provider` and `context_window`. A string provider also
requires a non-empty string `model`. A provider call accepts named arguments
only, rejects duplicate argument names, and obtains its model from either a
non-empty string `model` argument or the agent's non-empty top-level `model`.

If present, `fallback` must be an array. Each entry must be a non-empty route
string or a provider call containing its own non-empty string `model`; a
fallback cannot inherit the primary agent's model.

A pipeline requires `agent`. Its value must name a top-level agent declaration.
A safety bound requires at least one of `require` or `policy`. Duplicate property
diagnostics point to the repeated occurrence; malformed values point to the
smallest actionable property or entry range.

Checks accumulate independent errors in deterministic declaration and rule
order so one invocation can repair multiple issues.

Name resolution, type, pipeline-contract, and exhaustiveness codes in the
`SYL2xxx` range are documented in
[`docs/semantic-analysis.md`](semantic-analysis.md).
