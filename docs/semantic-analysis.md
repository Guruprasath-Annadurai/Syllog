# Syllog Semantic Analysis

`syllog-semantic` runs after the Pest parser and domain configuration validator.
It uses separate type and value namespaces and performs three ordered passes:

1. Collect global declarations, allowing forward references and diagnosing
   duplicates.
2. Resolve struct fields, enum payloads, state slots, function signatures,
   agent contracts, and pipeline signatures.
3. Resolve local parameters and `let` bindings, infer expressions, check calls,
   validate pipeline contracts, and prove supported matches exhaustive.

The public `SymbolTable` exposes built-in and declared type/value symbols.
`ResolvedType` represents primitives, structs, enums, state types, arrays,
tuples, functions, `Option<T>`, and `Result<T,E>`. An error sentinel suppresses
cascading diagnostics after an unresolved type or name.

## Implemented semantic diagnostics

| Code | Meaning |
| --- | --- |
| `SYL2001` | Duplicate symbol in the type or value namespace. |
| `SYL2002` | Unknown type name. |
| `SYL2003` | Unknown value, constructor, variant, or field. |
| `SYL2004` | Wrong generic type argument count. |
| `SYL2101` | Expression, argument, return, initializer, or pattern type mismatch. |
| `SYL2201` | Pipeline input/output incompatible with its selected agent contract. |
| `SYL2301` | Non-exhaustive closed match. |

## Algebraic types and match coverage

User enums are closed tagged unions. `Option<T>` has `some(T)` and `none`;
`Result<T,E>` has `ok(T)` and `err(E)`. Constructor calls and patterns validate
payload counts and types. Exhaustiveness is proven for user enums, `Option`,
`Result`, and `Bool`. Unguarded wildcard or binding arms cover the remainder;
guarded arms do not count as total coverage.

## Pipeline contracts

An agent may declare typed `input` and `output` properties. A pipeline selecting
that agent must give its first parameter the agent input type and its return type
the agent output type. A typed `result` property must match the pipeline return,
and its value expression is checked in a scope containing pipeline parameters.
Agents without typed input/output properties remain valid but provide no contract
to compare.

## Current boundary

This milestone does not implement user-defined generics, traits, overloads,
method lookup, implicit conversions, borrow checking, effects, record literals,
or cross-file modules. Exhaustiveness does not yet diagnose unreachable arms or
decompose nested patterns into a full pattern matrix. Integer literals are
compatible with explicit integer targets, but conversions between concrete
numeric types remain forbidden.
