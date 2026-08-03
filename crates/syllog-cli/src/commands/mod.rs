//! CLI compilation commands sharing the production compiler pipeline.

pub mod build;
pub mod dev;
pub mod new;
pub mod run;

use std::fs;
use std::path::Path;

use anyhow::Context;
use syllog_compiler::{compile, lower_to_hir, lower_to_mir, render_human};

pub(crate) fn compile_to_mir(path: &Path) -> anyhow::Result<Option<syllog_ir::MirProgram>> {
    let source =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let compilation = compile(path.display().to_string(), &source);
    if !compilation.success() {
        eprint!("{}", render_human(&source, &compilation.diagnostics));
        return Ok(None);
    }
    let ast = compilation
        .ast
        .as_ref()
        .expect("successful compilation has AST");
    let symbols = compilation
        .symbols
        .as_ref()
        .expect("successful compilation has symbols");
    let hir = lower_to_hir(ast, symbols)
        .map_err(|diagnostics| anyhow::anyhow!("HIR lowering failed: {diagnostics:#?}"))?;
    let mir = lower_to_mir(&hir)
        .map_err(|diagnostics| anyhow::anyhow!("MIR lowering failed: {diagnostics:#?}"))?;
    Ok(Some(mir))
}
