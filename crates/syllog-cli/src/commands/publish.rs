//! Deterministic `syllog publish --dry-run` package assembly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, bail};
use semver::Version;
use syllog_registry_client::{
    ArchiveFile, HttpRegistryClient, PackageArchive, ProvenanceStatement, PublisherIdentity,
    RegistryCredential,
};

/// Validates and assembles a canonical package without network or publication.
pub fn execute(start: &Path) -> anyhow::Result<ExitCode> {
    let archive = assemble(start)?;
    print_report(&archive, true, None)?;
    Ok(ExitCode::SUCCESS)
}

/// Publishes a signed archive and its provenance through the network registry.
pub async fn execute_remote(
    start: &Path,
    registry: &str,
    nonce: &str,
    source_revision: &str,
    token: String,
    seed: [u8; 32],
) -> anyhow::Result<ExitCode> {
    let archive = assemble(start)?;
    let checksum = archive.checksum()?;
    let identity = PublisherIdentity::from_seed(archive.name.clone(), seed);
    let publication = identity.sign(archive.clone(), nonce)?;
    let provenance = ProvenanceStatement {
        schema_version: 1,
        archive_checksum: checksum,
        compiler: format!("syllog {}", env!("CARGO_PKG_VERSION")),
        source_revision: source_revision.into(),
    };
    let client = HttpRegistryClient::new(
        registry,
        RegistryCredential::new(token),
        Duration::from_secs(60),
    )?;
    let receipt = client.publish(&publication, &provenance).await?;
    print_report(&archive, false, Some(&receipt.provenance_id))?;
    Ok(ExitCode::SUCCESS)
}

fn assemble(start: &Path) -> anyhow::Result<PackageArchive> {
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let version = Version::parse(&project.manifest.package.version)
        .context("package version is not semantic version syntax")?;
    let mut files = vec![ArchiveFile::new(
        "Syllog.toml",
        std::fs::read(&project.manifest_path).context("could not read manifest")?,
    )];
    let mut included = BTreeSet::new();
    for target in &project.manifest.targets {
        let root = target.path.parent().unwrap_or(&project.root);
        for path in source_files(root)? {
            let relative = path
                .strip_prefix(&project.root)
                .context("source escaped project root after validation")?
                .to_str()
                .context("source path is not portable UTF-8")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if relative == "Syllog.toml" {
                bail!("source path collides with package manifest");
            }
            if included.insert(relative.clone()) {
                files.push(ArchiveFile::new(
                    relative,
                    std::fs::read(&path)
                        .with_context(|| format!("could not read {}", path.display()))?,
                ));
            }
        }
    }
    let dependencies = project
        .manifest
        .dependencies
        .iter()
        .map(|(name, dependency)| (name.clone(), dependency.requirement.clone()))
        .collect();
    PackageArchive::new(project.manifest.package.name, version, files, dependencies)
        .map_err(Into::into)
}

fn source_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                directories.push(entry.path());
            } else if file_type.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "syl")
            {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn print_report(
    archive: &PackageArchive,
    dry_run: bool,
    provenance_id: Option<&str>,
) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "dry_run": dry_run,
            "name": archive.name,
            "version": archive.version,
            "checksum": archive.checksum()?,
            "provenance_id": provenance_id,
            "files": archive.files.iter().map(|file| &file.path).collect::<Vec<_>>()
        }))?
    );
    Ok(())
}
