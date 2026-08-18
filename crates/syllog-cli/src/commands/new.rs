//! `syllog new` deterministic project scaffolding.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};

const BASIC_MANIFEST: &str = include_str!("../../../syllog-templates/basic/Syllog.toml");
const BASIC_SOURCE: &str = include_str!("../../../syllog-templates/basic/src/main.syl");
const AGENT_MANIFEST: &str = include_str!("../../../syllog-templates/agent/Syllog.toml");
const AGENT_SOURCE: &str = include_str!("../../../syllog-templates/agent/src/main.syl");
const NATIVE_MANIFEST: &str = include_str!("../../../syllog-templates/native/Syllog.toml");
const NATIVE_SOURCE: &str = include_str!("../../../syllog-templates/native/src/main.syl");

struct Template {
    name: &'static str,
    version: u32,
    manifest: &'static str,
    source: &'static str,
}

/// Creates a project below `parent` and publishes it with one atomic rename.
pub fn execute(parent: &Path, package_name: &str, template_name: &str) -> anyhow::Result<ExitCode> {
    validate_package_name(package_name)?;
    let template = template(template_name)?;
    let destination = parent.join(package_name);
    if destination.exists() {
        bail!(
            "refusing to overwrite existing target {}",
            destination.display()
        );
    }

    let staging = tempfile::Builder::new()
        .prefix(".syllog-new-")
        .tempdir_in(parent)
        .with_context(|| format!("could not create staging directory in {}", parent.display()))?;
    fs::create_dir(staging.path().join("src")).context("could not create source directory")?;
    fs::write(
        staging.path().join("Syllog.toml"),
        canonical_template_text(&template.manifest.replace("{{name}}", package_name)),
    )
    .context("could not write project manifest")?;
    fs::write(
        staging.path().join("src/main.syl"),
        canonical_template_text(template.source),
    )
    .context("could not write project source")?;
    fs::write(
        staging.path().join(".syllog-template-version"),
        format!("{}@{}\n", template.name, template.version),
    )
    .context("could not record template version")?;

    let staging_path = staging.keep();
    fs::rename(&staging_path, &destination).with_context(|| {
        format!(
            "could not atomically publish {} as {}",
            staging_path.display(),
            destination.display()
        )
    })?;
    println!(
        "created {} ({template_name}@{})",
        destination.display(),
        template.version
    );
    Ok(ExitCode::SUCCESS)
}

/// Produces byte-identical scaffolds even when a source checkout rewrites line endings.
fn canonical_template_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn validate_package_name(name: &str) -> anyhow::Result<()> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let valid_rest = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if !valid_start || !valid_rest || name.ends_with('-') || name.contains("--") {
        bail!(
            "invalid package name '{name}'; use lowercase ASCII letters, digits, and single hyphens"
        );
    }
    Ok(())
}

fn template(name: &str) -> anyhow::Result<Template> {
    match name {
        "basic" => Ok(Template {
            name: "basic",
            version: 1,
            manifest: BASIC_MANIFEST,
            source: BASIC_SOURCE,
        }),
        "agent" => Ok(Template {
            name: "agent",
            version: 1,
            manifest: AGENT_MANIFEST,
            source: AGENT_SOURCE,
        }),
        "native" => Ok(Template {
            name: "native",
            version: 1,
            manifest: NATIVE_MANIFEST,
            source: NATIVE_SOURCE,
        }),
        _ => bail!("unknown template '{name}'; expected basic, agent, or native"),
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_template_text;

    #[test]
    fn template_text_has_platform_independent_line_endings() {
        assert_eq!(
            canonical_template_text("first\r\nsecond\rthird\n"),
            "first\nsecond\nthird\n"
        );
    }
}
