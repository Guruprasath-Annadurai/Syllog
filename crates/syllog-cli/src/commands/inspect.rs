//! `syllog inspect` deterministic compiler and authority views.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};

/// Inspects project configuration, typed HIR, or capability authority.
pub fn execute(start: &Path, subject: &str, json: bool) -> anyhow::Result<ExitCode> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let value = match subject {
        "project" => serde_json::json!({
            "schema_version": 1,
            "root": project.root,
            "manifest_path": project.manifest_path,
            "manifest": project.manifest,
            "configuration_source": "Syllog.toml"
        }),
        "capabilities" => serde_json::json!({
            "schema_version": 1,
            "profile": project.manifest.capabilities,
            "explanation": [
                "Only listed environment variable names are visible; values are never inspected.",
                "Network endpoints require exact manifest grants.",
                "Runtime linear memory is capped by max_memory_bytes."
            ]
        }),
        "hir" => inspect_hir(&project)?,
        _ => bail!("unknown inspection '{subject}'; expected project, hir, or capabilities"),
    };
    println!(
        "{}",
        if json {
            serde_json::to_string(&value)?
        } else {
            serde_json::to_string_pretty(&value)?
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn inspect_hir(project: &syllog_project::Project) -> anyhow::Result<serde_json::Value> {
    let target = project
        .manifest
        .targets
        .first()
        .context("project has no target to inspect")?;
    let source = std::fs::read_to_string(&target.path)
        .with_context(|| format!("could not read {}", target.path.display()))?;
    let compilation = syllog_compiler::compile(target.path.display().to_string(), &source);
    if !compilation.success() {
        bail!(
            "target did not compile:\n{}",
            syllog_compiler::render_human(&source, &compilation.diagnostics)
        );
    }
    let hir = syllog_compiler::lower_to_hir(
        compilation
            .ast
            .as_ref()
            .expect("successful compile has AST"),
        compilation
            .symbols
            .as_ref()
            .expect("successful compile has symbols"),
    )
    .map_err(|diagnostics| anyhow::anyhow!("HIR lowering failed: {diagnostics:?}"))?;
    Ok(serde_json::json!({
        "schema_version": 1,
        "target": target.name,
        "hir": hir
    }))
}
