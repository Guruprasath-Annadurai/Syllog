//! Command-line entry point for the Syllog compiler front end.

mod commands;

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
        "check" => execute_check(&mut args),
        "build" => execute_build(&mut args),
        "run" => execute_run(&mut args),
        "dev" => execute_dev(&mut args),
        "schema" => execute_schema(&mut args),
        "new" => execute_new(&mut args),
        "help" | "--help" | "-h" => {
            println!(
                "Syllog compiler\n\nUSAGE:\n    syllog new NAME [--template basic|agent|native]\n    syllog check <file.syl> [--json|--diagnostic-format=json]\n    syllog dev [--json-events] [--once]\n    syllog build <file.syl> --target wasm32-syllog --output PATH\n    syllog run <file.syl> [--fuel N] [--memory-bytes N]\n    syllog schema manifest"
            );
            Ok(ExitCode::SUCCESS)
        }
        other => {
            bail!("unknown command '{other}'; expected new, check, dev, build, run, or schema")
        }
    }
}

fn execute_dev(args: &mut impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let mut json_events = false;
    let mut once = false;
    for argument in args {
        match argument.as_str() {
            "--json-events" => json_events = true,
            "--once" => once = true,
            _ => bail!("unknown dev option '{argument}'"),
        }
    }
    commands::dev::execute(&env::current_dir()?, json_events, once)
}

fn execute_check(args: &mut impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let path = args
        .next()
        .context("usage: syllog check <file.syl> [--json]")?;
    let mut format = DiagnosticFormat::Human;
    for argument in args {
        format = match argument.as_str() {
            "--json" | "--diagnostic-format=json" => DiagnosticFormat::Json,
            "--diagnostic-format=human" => DiagnosticFormat::Human,
            _ => bail!("unknown option '{argument}'; expected --json or --diagnostic-format=json"),
        };
    }
    check_file(Path::new(&path), format)
}

fn execute_build(args: &mut impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let path = args
        .next()
        .context("usage: syllog build <file.syl> --target wasm32-syllog --output PATH")?;
    let mut target = None;
    let mut output = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--target" => target = args.next(),
            "--output" | "-o" => output = args.next(),
            _ => bail!("unknown build option '{argument}'"),
        }
    }
    commands::build::execute(
        Path::new(&path),
        target.as_deref().unwrap_or("wasm32-syllog"),
        Path::new(output.as_deref().context("build requires --output PATH")?),
    )
}

fn execute_run(args: &mut impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let path = args
        .next()
        .context("usage: syllog run <file.syl> [--fuel N] [--memory-bytes N]")?;
    let mut fuel = 1_000_000_u64;
    let mut memory_bytes = 64 * 1024 * 1024_usize;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fuel" => {
                fuel = args
                    .next()
                    .context("--fuel requires an integer")?
                    .parse()
                    .context("invalid --fuel value")?;
            }
            "--memory-bytes" => {
                memory_bytes = args
                    .next()
                    .context("--memory-bytes requires an integer")?
                    .parse()
                    .context("invalid --memory-bytes value")?;
            }
            _ => bail!("unknown run option '{argument}'"),
        }
    }
    commands::run::execute(Path::new(&path), fuel, memory_bytes)
}

fn execute_schema(args: &mut impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let schema = args.next().context("usage: syllog schema manifest")?;
    if schema != "manifest" {
        bail!("unknown schema '{schema}'; expected manifest");
    }
    if let Some(argument) = args.next() {
        bail!("unexpected schema argument '{argument}'");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&syllog_project::manifest_schema())?
    );
    Ok(ExitCode::SUCCESS)
}

fn execute_new(args: &mut impl Iterator<Item = String>) -> anyhow::Result<ExitCode> {
    let name = args
        .next()
        .context("usage: syllog new NAME [--template basic|agent|native]")?;
    let mut template = "basic".to_owned();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--template" => template = args.next().context("--template requires a name")?,
            _ => bail!("unknown new option '{argument}'"),
        }
    }
    commands::new::execute(&env::current_dir()?, &name, &template)
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
