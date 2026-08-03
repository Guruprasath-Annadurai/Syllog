//! `syllog dev` structured incremental development loop.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;
use syllog_dev_server::{DevEvent, DevOptions, serve};

/// Discovers and serves a project until interrupted or the initial `--once` build completes.
pub fn execute(start: &Path, json_events: bool, once: bool) -> anyhow::Result<ExitCode> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not initialize development runtime")?;
    runtime.block_on(run(start, json_events, once))
}

async fn run(start: &Path, json_events: bool, once: bool) -> anyhow::Result<ExitCode> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let target_count = project.manifest.targets.len();
    let mut handle = serve(project, DevOptions::default())
        .await
        .context("could not start development service")?;
    let mut completed = 0_usize;
    loop {
        tokio::select! {
            event = handle.next_event() => {
                let Some(event) = event else { break };
                render_event(&event, json_events)?;
                completed += terminal_builds(&event);
                if once && completed >= target_count {
                    break;
                }
            }
            result = tokio::signal::ctrl_c(), if !once => {
                result.context("could not listen for interrupt")?;
                break;
            }
        }
    }
    handle
        .shutdown()
        .await
        .context("development service shutdown failed")?;
    Ok(ExitCode::SUCCESS)
}

fn terminal_builds(event: &DevEvent) -> usize {
    match event {
        DevEvent::RuntimeRestarted => 1,
        DevEvent::Diagnostics(report) if !report.success => 1,
        _ => 0,
    }
}

fn render_event(event: &DevEvent, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(event)?);
        return Ok(());
    }
    match event {
        DevEvent::Building(id) => println!("building #{}", id.0),
        DevEvent::Diagnostics(report) if report.success => println!("checked successfully"),
        DevEvent::Diagnostics(report) => {
            for diagnostic in &report.diagnostics {
                eprintln!(
                    "{}:{}:{}: {}[{}]: {}",
                    diagnostic.file,
                    diagnostic.range.start.line + 1,
                    diagnostic.range.start.column + 1,
                    diagnostic.severity,
                    diagnostic.code,
                    diagnostic.message
                );
            }
        }
        DevEvent::Ready(artifact) => println!(
            "ready {} {}",
            artifact.target,
            &artifact.digest[..12.min(artifact.digest.len())]
        ),
        DevEvent::RuntimeRestarted => println!("runtime restarted"),
    }
    Ok(())
}
