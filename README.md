# Syllog

A small, statically-typed programming language, built from scratch in Rust.

**Author:** Guruprasath Annadurai

## Status

Early development. Current milestone: **lexer** (tokenizer with source-span
tracking and multi-error reporting). See [`docs/design.md`](docs/design.md)
for the full design and [`docs/grammar.ebnf`](docs/grammar.ebnf) for the
grammar.

## Try it

```bash
cargo run -- examples/hello.syl
```

## Roadmap

- [x] Lexer
- [ ] Parser & AST
- [ ] Static type checker
- [ ] Bytecode compiler & VM
- [ ] Standard library
- [ ] CLI tooling (REPL, fmt, test runner)
- [ ] Editor support (Tree-sitter grammar + LSP)

## License

MIT — see [LICENSE](LICENSE).
