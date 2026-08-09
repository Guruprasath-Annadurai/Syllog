//! `syllog run` implementation.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;
use syllog_codegen_wasm::{WasmOptions, emit_with_async_frames};
use syllog_runtime::{Sandbox, SandboxPolicy};

/// Compiles and executes one source file in the production Wasm sandbox.
pub fn execute(path: &Path, fuel: u64, memory_bytes: usize) -> anyhow::Result<ExitCode> {
    let Some(program) = super::compile_program(path)? else {
        return Ok(ExitCode::FAILURE);
    };
    let artifact =
        emit_with_async_frames(&program.mir, &program.async_frames, &WasmOptions::default())
            .context("Wasm code generation failed")?;
    let policy = SandboxPolicy::new(fuel, memory_bytes).context("invalid sandbox policy")?;
    let result = Sandbox::new()
        .context("could not initialize Wasm sandbox")?
        .execute_i64(&artifact.bytes, "main", &policy)
        .context("sandboxed program failed")?;
    println!("{result}");
    Ok(ExitCode::SUCCESS)
}
