//! Command-line entry point for the Syllog compiler front end.

use anyhow::{Context, bail};
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use syllog_compiler::{EditorReport, compile, render_human};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticFormat {
    Human,
    Json,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    match execute(env::args().skip(1)) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn execute(mut args: impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let command = args.next().unwrap_or_else(|| "help".to_owned());
    match command.as_str() {
        "check" | "run" => {
            let path = args
                .next()
                .with_context(|| format!("usage: syllog {command} <file.syl> [--json]"))?;
            let mut format = DiagnosticFormat::Human;
            for argument in args {
                format = match argument.as_str() {
                    "--json" | "--diagnostic-format=json" => DiagnosticFormat::Json,
                    "--diagnostic-format=human" => DiagnosticFormat::Human,
                    _ => bail!(
                        "unknown option '{argument}'; expected --json or --diagnostic-format=json"
                    ),
                };
            }
            check_file(Path::new(&path), format)
        }
        "help" | "--help" | "-h" => {
            println!(
                "Syllog compiler\n\nUSAGE:\n    syllog check <file.syl> [--json|--diagnostic-format=json]\n    syllog run <file.syl> [--json|--diagnostic-format=json]"
            );
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("unknown command '{other}'; expected 'check' or 'run'"),
    }
}

fn check_file(path: &Path, format: DiagnosticFormat) -> anyhow::Result<ExitCode> {
    let source =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let compilation = compile(path.display().to_string(), &source);

    match format {
        DiagnosticFormat::Human => {
            if !compilation.diagnostics.is_empty() {
                eprint!("{}", render_human(&source, &compilation.diagnostics));
            }
            if compilation.success() {
                let declaration_count = compilation.ast.as_ref().map_or(0, |ast| ast.items.len());
                println!(
                    "checked {} ({declaration_count} declarations; parse → resolve → type-check)",
                    path.display()
                );
            }
        }
        DiagnosticFormat::Json => {
            let report = EditorReport::from(&compilation);
            println!(
                "{}",
                serde_json::to_string(&report).context("could not serialize JSON diagnostics")?
            );
        }
    }

    Ok(if compilation.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
