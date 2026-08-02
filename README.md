# Syllog

A small, statically-typed programming language, built from scratch in Rust.

**Author:** Guruprasath Annadurai

## Status

Early development. Current milestone: **parser & AST** (recursive-descent +
precedence climbing, with multi-error recovery on top of the lexer's).
See [`docs/design.md`](docs/design.md) for the full design and
[`docs/grammar.ebnf`](docs/grammar.ebnf) for the grammar.

## Try it

```bash
cargo run -- examples/hello.syl            # parses and lists top-level items
cargo run -- examples/hello.syl --tokens   # dumps the raw token stream
```

## Roadmap

- [x] Lexer
- [x] Parser & AST
- [ ] Static type checker
- [ ] Bytecode compiler & VM
- [ ] Standard library
- [ ] CLI tooling (REPL, fmt, test runner)
- [ ] Editor support (Tree-sitter grammar + LSP)

## License

MIT — see [LICENSE](LICENSE).
