//! Deterministic `syllog publish --dry-run` package assembly.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use semver::Version;
use syllog_registry_client::{ArchiveFile, PackageArchive};

/// Validates and assembles a canonical package without network or publication.
pub fn execute(start: &Path) -> anyhow::Result<ExitCode> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let version = Version::parse(&project.manifest.package.version)
        .context("package version is not semantic version syntax")?;
    let mut files = vec![ArchiveFile::new(
        "Syllog.toml",
        std::fs::read(&project.manifest_path).context("could not read manifest")?,
    )];
    for target in &project.manifest.targets {
        let relative = target
            .path
            .strip_prefix(&project.root)
            .context("target escaped project root after validation")?;
        let relative = relative
            .to_str()
            .context("target path is not portable UTF-8")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative == "Syllog.toml" {
            bail!("target path collides with package manifest");
        }
        files.push(ArchiveFile::new(
            relative,
            std::fs::read(&target.path)
                .with_context(|| format!("could not read {}", target.path.display()))?,
        ));
    }
    let dependencies = project
        .manifest
        .dependencies
        .iter()
        .map(|(name, dependency)| (name.clone(), dependency.requirement.clone()))
        .collect();
    let archive = PackageArchive::new(project.manifest.package.name, version, files, dependencies)?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "dry_run": true,
            "name": archive.name,
            "version": archive.version,
            "checksum": archive.checksum()?,
            "files": archive.files.iter().map(|file| &file.path).collect::<Vec<_>>()
        }))?
    );
    Ok(ExitCode::SUCCESS)
}
