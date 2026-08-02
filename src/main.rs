mod ast;
mod lexer;
mod parser;
mod span;
mod token;

use ast::Item;
use lexer::Lexer;
use parser::Parser;
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: syllog <file.syl>");
        return ExitCode::FAILURE;
    };

    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read '{path}': {e}");
            return ExitCode::FAILURE;
        }
    };

    let (tokens, errors) = Lexer::new(&source).tokenize();

    if !errors.is_empty() {
        for err in &errors {
            eprintln!(
                "error: {} at line {}, col {}",
                err.message, err.span.line, err.span.col
            );
        }
        return ExitCode::FAILURE;
    }

    let show_tokens = args.iter().any(|a| a == "--tokens");
    if show_tokens {
        for token in &tokens {
            println!("{:>4}:{:<4} {:?}", token.span.line, token.span.col, token.kind);
        }
        return ExitCode::SUCCESS;
    }

    let (program, parse_errors) = Parser::new(tokens).parse_program();

    if !parse_errors.is_empty() {
        for err in &parse_errors {
            eprintln!(
                "error: {} (line {}, col {})",
                err.message, err.span.line, err.span.col
            );
        }
        return ExitCode::FAILURE;
    }

    println!("parsed {} item(s):", program.items.len());
    for item in &program.items {
        match item {
            Item::Function(f) => println!(
                "  fn {}({} params) -> {}",
                f.name,
                f.params.len(),
                f.return_type.as_deref().unwrap_or("()")
            ),
            Item::Let(l) => println!("  let {}", l.name),
        }
    }

    ExitCode::SUCCESS
}
