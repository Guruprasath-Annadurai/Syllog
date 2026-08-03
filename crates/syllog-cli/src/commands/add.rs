//! Atomic `syllog add` manifest editing.

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use semver::VersionReq;
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, Item, Table, value};

/// Adds or updates one dependency without discarding unrelated manifest text.
pub fn execute(start: &Path, specification: &str) -> anyhow::Result<ExitCode> {
    let (name, requirement) = specification
        .rsplit_once('@')
        .filter(|(name, requirement)| !name.is_empty() && !requirement.is_empty())
        .context("dependency must use NAME@RANGE syntax")?;
    validate_name(name)?;
    VersionReq::parse(requirement).context("invalid dependency version range")?;
    let project = syllog_project::discover(start).context("could not discover Syllog project")?;
    let source = std::fs::read_to_string(&project.manifest_path)
        .context("could not read project manifest")?;
    let mut document = source
        .parse::<DocumentMut>()
        .context("could not edit project manifest")?;
    if document.get("dependencies").is_none() {
        document["dependencies"] = Item::Table(Table::new());
    }
    let dependencies = document["dependencies"]
        .as_table_mut()
        .context("manifest dependencies must be a table")?;
    dependencies[name] = value(requirement);
    dependencies.set_implicit(false);

    let parent = project
        .manifest_path
        .parent()
        .context("manifest has no parent directory")?;
    let mut temporary = NamedTempFile::new_in(parent).context("could not stage manifest edit")?;
    temporary
        .write_all(document.to_string().as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .context("could not durably stage manifest edit")?;
    temporary
        .persist(&project.manifest_path)
        .map_err(|error| error.error)
        .context("could not atomically replace manifest")?;
    println!("added {name}@{requirement}");
    Ok(ExitCode::SUCCESS)
}

fn validate_name(name: &str) -> anyhow::Result<()> {
    let mut characters = name.chars();
    if characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !name.ends_with('-')
        && !name.contains("--")
    {
        Ok(())
    } else {
        bail!("invalid package name '{name}'")
    }
}
