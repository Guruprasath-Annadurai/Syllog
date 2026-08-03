//! `syllog build` implementation.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use syllog_codegen_wasm::{WasmOptions, emit};

/// Builds one source file into a deterministic Wasm artifact.
pub fn execute(path: &Path, target: &str, output: &Path) -> anyhow::Result<ExitCode> {
    if target != "wasm32-syllog" {
        bail!("unsupported target '{target}'; expected wasm32-syllog");
    }
    let Some(mir) = super::compile_to_mir(path)? else {
        return Ok(ExitCode::FAILURE);
    };
    let artifact = emit(&mir, &WasmOptions::default()).context("Wasm code generation failed")?;
    fs::write(output, artifact.bytes)
        .with_context(|| format!("could not write {}", output.display()))?;
    println!("built {} -> {}", path.display(), output.display());
    Ok(ExitCode::SUCCESS)
}
