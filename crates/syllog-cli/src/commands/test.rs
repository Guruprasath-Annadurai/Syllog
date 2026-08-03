//! `syllog test` isolated source-level test execution.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use serde::Serialize;
use syllog_compiler::hir::HirDefinitionKind;
use syllog_runtime::{Sandbox, SandboxPolicy};
use syllog_semantic::{PrimitiveType, ResolvedType};

#[derive(Serialize)]
struct TestReport {
    schema_version: u32,
    success: bool,
    tests: Vec<TestResult>,
}

#[derive(Serialize)]
struct TestResult {
    target: String,
    name: String,
    status: &'static str,
}

struct DiscoveredTest {
    target: String,
    name: String,
    entry: syllog_ir::DefId,
    returns_bool: bool,
    program: syllog_ir::MirProgram,
    memory_bytes: usize,
}

/// Discovers `#[test]` functions and executes each in a fresh Wasm sandbox.
pub fn execute(start: &Path, json: bool) -> anyhow::Result<ExitCode> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let mut tests = Vec::new();
    for target in &project.manifest.targets {
        tests.extend(discover_target_tests(
            target,
            project.manifest.capabilities.max_memory_bytes,
        )?);
    }
    tests.sort_by(|left, right| (&left.target, &left.name).cmp(&(&right.target, &right.name)));
    let results = tests
        .into_iter()
        .map(run_test)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let success = results.iter().all(|result| result.status == "passed");
    let report = TestReport {
        schema_version: 1,
        success,
        tests: results,
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        for test in &report.tests {
            println!("{} ... {}", test.name, test.status);
        }
        println!(
            "test result: {}. {} tests",
            if success { "ok" } else { "FAILED" },
            report.tests.len()
        );
    }
    Ok(if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn discover_target_tests(
    target: &syllog_project::Target,
    memory_bytes: u64,
) -> anyhow::Result<Vec<DiscoveredTest>> {
    let source = std::fs::read_to_string(&target.path)
        .with_context(|| format!("could not read {}", target.path.display()))?;
    let compilation = syllog_compiler::compile(target.path.display().to_string(), &source);
    if !compilation.success() {
        bail!(
            "test target {} did not compile:\n{}",
            target.name,
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
    .map_err(|diagnostics| anyhow::anyhow!("test HIR lowering failed: {diagnostics:?}"))?;
    let mir = syllog_compiler::lower_to_mir(&hir)
        .map_err(|diagnostics| anyhow::anyhow!("test MIR lowering failed: {diagnostics:?}"))?;
    let mut tests = Vec::new();
    for definition in hir.modules.iter().flat_map(|module| &module.definitions) {
        let HirDefinitionKind::Function(function) = &definition.kind else {
            continue;
        };
        if !function.is_test {
            continue;
        }
        if !function.parameters.is_empty() {
            bail!("test '{}' must not declare parameters", definition.name);
        }
        let returns_bool = match &function.result {
            ResolvedType::Primitive(PrimitiveType::Bool) => true,
            ResolvedType::Unit => false,
            other => bail!(
                "test '{}' must return Bool or (), found {other}",
                definition.name
            ),
        };
        tests.push(DiscoveredTest {
            target: target.name.clone(),
            name: definition.name.clone(),
            entry: syllog_ir::DefId {
                module: definition.id.module.0,
                index: definition.id.index,
            },
            returns_bool,
            program: mir.clone(),
            memory_bytes: usize::try_from(memory_bytes)
                .context("test memory limit does not fit this platform")?,
        });
    }
    Ok(tests)
}

fn run_test(mut test: DiscoveredTest) -> anyhow::Result<TestResult> {
    test.program.entry = Some(test.entry);
    let artifact =
        syllog_codegen_wasm::emit(&test.program, &syllog_codegen_wasm::WasmOptions::default())
            .with_context(|| format!("could not compile test '{}'", test.name))?;
    let policy = SandboxPolicy::new(1_000_000, test.memory_bytes)
        .context("invalid project test sandbox policy")?;
    let value = Sandbox::new()
        .context("could not initialize test sandbox")?
        .execute_i64(&artifact.bytes, "main", &policy)
        .with_context(|| format!("test '{}' trapped", test.name))?;
    let passed = !test.returns_bool || value != 0;
    Ok(TestResult {
        target: test.target,
        name: test.name,
        status: if passed { "passed" } else { "failed" },
    })
}
