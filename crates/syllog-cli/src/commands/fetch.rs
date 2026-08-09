//! Resumable, checksum-verified retrieval of all locked packages.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use syllog_package::{ContentAddressedCache, read_lockfile};
use syllog_registry_client::{HttpRegistryClient, RegistryCredential};

/// Fetches every missing lockfile archive into the immutable local cache.
pub async fn execute(start: &Path, registry: &str, token: String) -> anyhow::Result<ExitCode> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let resolution = read_lockfile(&project.root.join("Syllog.lock"))?;
    if resolution.format != 1 {
        anyhow::bail!("unsupported Syllog.lock format {}", resolution.format);
    }
    super::validate_lock_graph(&project, &resolution)?;
    let cache = ContentAddressedCache::new(project.root.join(".syllog/cache"));
    let client = HttpRegistryClient::new(
        registry,
        RegistryCredential::new(token),
        Duration::from_secs(120),
    )?;
    let mut downloaded = 0_usize;
    for package in &resolution.packages {
        let destination = cache.path_for(&package.checksum)?;
        if destination.exists() {
            cache.load(&package.checksum).with_context(|| {
                format!(
                    "cached package {} failed integrity verification",
                    package.name
                )
            })?;
            continue;
        }
        client
            .download_resumable(
                &package.name,
                &package.version,
                &package.checksum,
                &destination,
            )
            .await?;
        cache.load(&package.checksum)?;
        downloaded += 1;
    }
    println!(
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "locked": resolution.packages.len(),
            "downloaded": downloaded,
            "cache_hits": resolution.packages.len() - downloaded
        })
    );
    Ok(ExitCode::SUCCESS)
}
