mod lexer;
mod span;
mod token;

use lexer::Lexer;
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

    for token in &tokens {
        println!("{:>4}:{:<4} {:?}", token.span.line, token.span.col, token.kind);
    }

    ExitCode::SUCCESS
}
