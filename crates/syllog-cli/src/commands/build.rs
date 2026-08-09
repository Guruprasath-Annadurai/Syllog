//! `syllog build` implementation.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use syllog_codegen_wasm::{WasmOptions, emit_with_async_frames};

/// Builds one source file into a deterministic Wasm artifact.
pub fn execute(path: &Path, target: &str, output: &Path) -> anyhow::Result<ExitCode> {
    if target != "wasm32-syllog" {
        bail!("unsupported target '{target}'; expected wasm32-syllog");
    }
    let Some(program) = super::compile_program(path)? else {
        return Ok(ExitCode::FAILURE);
    };
    let artifact =
        emit_with_async_frames(&program.mir, &program.async_frames, &WasmOptions::default())
            .context("Wasm code generation failed")?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    fs::write(output, artifact.bytes)
        .with_context(|| format!("could not write {}", output.display()))?;
    println!("built {} -> {}", path.display(), output.display());
    Ok(ExitCode::SUCCESS)
}
