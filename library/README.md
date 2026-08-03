# Syllog standard library contract (v0.1)

The checked-in standard library is source code compiled by the same frontend as
application code. `api-v1.json` is the compatibility snapshot: removals or
signature changes require an explicit compatibility review and snapshot version
change.

## Layering and authority

- `core` has no package dependency, host import, allocator, or capability.
- `alloc` depends only on `core` and defines checked allocation metadata.
- `io` depends on `core` and `alloc`; authority-bearing functions require an
  `IoCapability` whose token is private to the defining module.
- `async` depends on `core` and `alloc`; scheduling functions require a
  `TaskCapability` whose token is private.
- `provider` depends on `core`, `alloc`, and `async`; provider entry points
  require a `ProviderCapability` whose token is private.

## Public function contracts

| Function | Success contract | Failure contract |
| --- | --- | --- |
| `core::min_i64` | Returns the lower operand. | Infallible. |
| `core::max_i64` | Returns the higher operand. | Infallible. |
| `core::clamp_i64` | Bounds a value inclusively. | Caller must supply an ordered interval; interval validation is not yet a distinct API. |
| `core::compare_i64` | Returns exactly one `Ordering` variant. | Infallible. |
| `alloc::checked_capacity` | Returns the requested capacity at or below the maximum. | Returns `ReserveError::capacity_exceeded`. |
| `io::validate_read` | Accepts a positive byte count with explicit I/O authority. | Returns `IoError::invalid_size` for zero; host denial is represented by `IoError::denied` at the adapter boundary. |
| `async::ready` | Produces an already-ready value with explicit task authority. | Infallible; scheduling failures belong to later scheduling APIs. |
| `provider::validate_request` | Accepts a non-empty request with explicit provider authority. | Returns `ProviderError::empty_request`; host denial is represented by `ProviderError::denied` at the adapter boundary. |

These are deliberately small bootstrap APIs, not claims of completed OS I/O,
allocation, scheduling, or provider adapters. Their host-backed implementations
remain gated by the async/runtime and native-backend phases.
