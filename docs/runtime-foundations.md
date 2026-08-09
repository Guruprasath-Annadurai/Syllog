# Runtime Foundations

This milestone establishes the execution boundaries required before dynamic
evolution or autonomous loops can be considered.

## Streaming provider execution

`syllog-proxy` defines `ProviderAdapter`, an object-safe asynchronous boundary
for model providers. Each invocation receives a `ModelRequest` and writes
ordered token events into an executor-owned Tokio channel. `PipelineExecutor`
requires a non-zero channel capacity and returns the receiving stream
immediately; a full channel suspends the provider task until the consumer makes
space. Dropping the receiver cancels further delivery without panicking.

`MockProvider::tokens` produces a deterministic successful stream.
`MockProvider::scripted` accepts token/error events and terminates at the first
error. Provider failures become the final ordered stream event, after any tokens
already delivered.

OpenAI and Anthropic adapters can use a bounded incremental HTTP/SSE transport.
It validates HTTPS except for loopback contract servers, installs
provider-specific authentication headers, enforces response/event limits, and
normalizes authentication, rate-limit, timeout, protocol, and availability
failures. `Retry-After` is retained as typed metadata; retry policy remains an
explicit pipeline decision. The local adapter supports an explicitly configured
child process or loopback TCP socket without invoking a shell.

The multi-stage executor provides serial and bounded fan-out groups, retry and
deadline policies, shared circuit breakers, structured cancellation, retained
failure lifecycle events, and supervised cleanup after a configurable grace
period. It does not place prompts, tokens, or provider credentials in lifecycle
events.

## Wasmtime sandbox policy

Every `execute_i32` invocation creates a fresh Wasmtime `Store`. The caller must
provide a non-zero fuel allowance and per-linear-memory byte limit:

- Fuel is installed before instantiation. Exhaustion returns the typed
  `SandboxError::FuelExhausted` error.
- Declared minimum memory is checked before instantiation. Runtime
  `memory.grow` is intercepted by a store resource limiter. Both violations
  return `SandboxError::MemoryLimitExceeded`.
- Host imports are denied by default. The only implemented grant is
  `HostCapability::LogI32`, corresponding exactly to
  `syllog::log_i32(i32)`. Unknown imports and known-but-ungranted imports return
  `SandboxError::CapabilityDenied` before instantiation.
- WASI, filesystem, environment, clock, randomness, sockets, native pointers,
  and process APIs are not linked.

The current typed call boundary is a no-argument Wasm export returning `i32`.
Memory limits apply to each linear memory; aggregate multi-memory accounting,
table limits, epoch deadlines, async host calls, output quotas, and persistent
instance lifecycle are future work.

## Deliberately deferred

There is no `evo` compilation, signing, promotion, hot-swap, or rollback path.
There is no `asi_loop` scheduler or recursive optimization lifecycle. Those
features must build on a broader typed ABI, artifact provenance, persistent
instance supervision, and adversarial sandbox tests rather than bypassing the
foundations above.
