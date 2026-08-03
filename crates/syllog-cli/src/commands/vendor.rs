//! Verified offline `syllog vendor` extraction.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use syllog_package::{ContentAddressedCache, read_lockfile};
use syllog_registry_client::PackageArchive;

/// Vendors every locked package from the local content-addressed cache.
pub fn execute(start: &Path) -> anyhow::Result<ExitCode> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let resolution = read_lockfile(&project.root.join("Syllog.lock"))?;
    let cache = ContentAddressedCache::new(project.root.join(".syllog/cache"));
    let destination = project.root.join("vendor");
    if destination.exists() {
        bail!("refusing to overwrite existing vendor directory");
    }
    let staging = tempfile::Builder::new()
        .prefix(".syllog-vendor-")
        .tempdir_in(&project.root)
        .context("could not create vendor staging directory")?;
    for package in &resolution.packages {
        let bytes = cache
            .load(&package.checksum)
            .with_context(|| format!("package {} is unavailable offline", package.name))?;
        let archive = PackageArchive::from_bytes(&bytes)?;
        if archive.name != package.name || archive.version != package.version {
            bail!(
                "cached archive identity does not match lockfile for {}",
                package.name
            );
        }
        let root = staging
            .path()
            .join(format!("{}-{}", package.name, package.version));
        for file in archive.files {
            let path = root.join(&file.path);
            std::fs::create_dir_all(path.parent().context("archive path has no parent")?)?;
            std::fs::write(&path, file.content)?;
        }
    }
    let staging_path = staging.keep();
    std::fs::rename(&staging_path, &destination)
        .context("could not atomically publish vendor directory")?;
    println!("vendored {} packages", resolution.packages.len());
    Ok(ExitCode::SUCCESS)
}
