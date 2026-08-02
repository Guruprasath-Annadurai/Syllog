# Syllog — Design Document

**Author:** Guruprasath Annadurai
**Status:** Draft (v0.1 scope)

## What Syllog is

Syllog is a small, statically-typed programming language, built from scratch
(hand-written lexer, parser, type checker, bytecode compiler, and VM — no
parser generators, no borrowed runtimes). It is influenced by Rust (explicit
types, expression-oriented syntax) and ML-family languages (type inference,
pattern matching), but makes its own tradeoffs rather than cloning either.

## What Syllog is not (yet)

To keep this honest and buildable, the following are explicitly **out of
scope** for v1.0 and are not claimed anywhere in this repo until they exist
as working, tested code:

- Native mobile UI generation
- Any claim of a fixed "accuracy" percentage — Syllog does not have an
  accuracy metric; if we publish a number (e.g. "type checker catches N% of
  a bug corpus"), it will link to the benchmark that produced it.
- Any interaction with the internal weights/activations of third-party LLMs
  accessed over an API — this is not technically possible for closed models
  and Syllog will never claim to do it.

## Core design goals

1. **Small, learnable core.** A programmer should be able to read the whole
   grammar in one sitting.
2. **Errors that teach.** Every compiler error carries a source span and a
   plain-English explanation, modeled on `rustc`/Elm diagnostics.
3. **Predictable performance.** Bytecode VM with a documented instruction
   set; no hidden allocations in hot paths.
4. **Self-hosting-capable standard library.** As much of the stdlib as
   possible is written in Syllog itself, once the language can support it.

## Planned (not yet built) directions

These are real, concrete engineering directions — not claims — to be
tackled after the core language (lexer → parser → type checker → VM) is
solid and tested:

- **LLM tooling DSL:** first-class syntax for typed calls to LLM APIs
  (`agent`, `prompt` as language constructs that compile to ordinary typed
  HTTP calls), plus a stdlib module for self-consistency checking (running
  a prompt N times and diffing outputs) as a real, non-fabricated substitute
  for "auditing" a closed model's reasoning.
- **Mobile target:** compiling Syllog business-logic modules to a library
  callable from Kotlin/Swift, or to WASM embeddable in a mobile shell. UI
  is out of scope — Syllog targets logic, not rendering.

## Grammar

See [`grammar.ebnf`](./grammar.ebnf).

## Example program (target syntax, not yet all implemented)

```syllog
fn fib(n: Int) -> Int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}

fn main() {
    let result = fib(10)
    print(result)
}
```
