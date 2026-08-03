//! Executable contracts for Syllog governance and language conformance.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;

const REQUIRED_DOCUMENTS: [&str; 4] = [
    "docs/governance/versioning.md",
    "docs/governance/rfc-process.md",
    "docs/governance/security.md",
    "docs/adr/0001-compiler-pipeline.md",
];

const REQUIRED_SECTIONS: [&str; 4] = [
    "## Status",
    "## Decision",
    "## Compatibility",
    "## Security impact",
];

/// One executable language conformance case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceCase {
    /// Language edition used to compile the source.
    pub edition: String,
    /// Normalized absolute source path.
    pub source: PathBuf,
    /// Required compiler or runtime outcome.
    pub expected: ExpectedOutcome,
}

/// Observable result required by a conformance case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedOutcome {
    /// Compilation must succeed without errors.
    Pass,
    /// Compilation must emit these stable diagnostic codes in order.
    Diagnostics(Vec<String>),
    /// Execution must produce exact stdout and process status.
    Run {
        /// Exact UTF-8 standard output.
        stdout: String,
        /// Expected process exit code.
        exit_code: i32,
    },
}

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    cases: Vec<ManifestCase>,
}

#[derive(Debug, Deserialize)]
struct ManifestCase {
    edition: String,
    source: PathBuf,
    expected: ManifestOutcome,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ManifestOutcome {
    Pass,
    Diagnostics { codes: Vec<String> },
    Run { stdout: String, exit_code: i32 },
}

/// Loads, validates, normalizes, and deterministically orders a conformance
/// manifest rooted at `spec/cases`.
///
/// # Errors
///
/// Returns an error for missing or malformed manifests, unsupported schemas,
/// unsafe or missing source paths, empty editions, duplicate cases, or empty
/// diagnostic codes.
pub fn load_cases(root: &Path) -> anyhow::Result<Vec<ConformanceCase>> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("could not resolve conformance root {}", root.display()))?;
    let manifest_path = root.join("manifest.json");
    let manifest_source = fs::read_to_string(&manifest_path)
        .with_context(|| format!("could not read {}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_str(&manifest_source)
        .with_context(|| format!("could not parse {}", manifest_path.display()))?;
    if manifest.schema_version != 1 {
        bail!(
            "unsupported conformance schema {}; expected 1",
            manifest.schema_version
        );
    }

    let mut identities = BTreeSet::new();
    let mut cases = Vec::with_capacity(manifest.cases.len());
    for case in manifest.cases {
        if case.edition.trim().is_empty() {
            bail!("conformance case edition cannot be empty");
        }
        if case.source.is_absolute()
            || case
                .source
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            || case
                .source
                .extension()
                .is_none_or(|extension| extension != "syl")
        {
            bail!("unsafe conformance source path {}", case.source.display());
        }
        let source = root.join(&case.source);
        if !source.is_file() {
            bail!("conformance source does not exist: {}", source.display());
        }
        if !identities.insert((case.edition.clone(), source.clone())) {
            bail!(
                "duplicate conformance case for edition {} and {}",
                case.edition,
                source.display()
            );
        }
        let expected = match case.expected {
            ManifestOutcome::Pass => ExpectedOutcome::Pass,
            ManifestOutcome::Diagnostics { codes } => {
                if codes.is_empty() || codes.iter().any(|code| code.trim().is_empty()) {
                    bail!("diagnostic conformance cases require non-empty codes");
                }
                ExpectedOutcome::Diagnostics(codes)
            }
            ManifestOutcome::Run { stdout, exit_code } => {
                ExpectedOutcome::Run { stdout, exit_code }
            }
        };
        cases.push(ConformanceCase {
            edition: case.edition,
            source,
            expected,
        });
    }
    cases.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.edition.cmp(&right.edition))
    });
    Ok(cases)
}

/// One missing governance document or required section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceIssue {
    /// Repository-relative Markdown path.
    pub path: PathBuf,
    /// Missing headings, or `document` when the file itself is absent.
    pub missing_sections: Vec<String>,
}

/// Validates required governance documents and every Markdown file in the
/// governance and architecture-decision directories.
///
/// # Errors
///
/// Returns an I/O error when an existing governance path cannot be read.
pub fn validate_governance(repository: &Path) -> io::Result<Vec<GovernanceIssue>> {
    let mut paths: BTreeSet<PathBuf> = REQUIRED_DOCUMENTS.iter().map(PathBuf::from).collect();
    for directory in ["docs/governance", "docs/adr"] {
        let absolute = repository.join(directory);
        match fs::read_dir(&absolute) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    if entry.file_type()?.is_file()
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "md")
                    {
                        paths.insert(Path::new(directory).join(entry.file_name()));
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    let mut issues = Vec::new();
    for relative in paths {
        let contents = match fs::read_to_string(repository.join(&relative)) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                issues.push(GovernanceIssue {
                    path: relative,
                    missing_sections: vec!["document".into()],
                });
                continue;
            }
            Err(error) => return Err(error),
        };
        let missing_sections = REQUIRED_SECTIONS
            .iter()
            .filter(|heading| !contents.lines().any(|line| line.trim() == **heading))
            .map(|heading| heading.trim_start_matches("## ").to_owned())
            .collect::<Vec<_>>();
        if !missing_sections.is_empty() {
            issues.push(GovernanceIssue {
                path: relative,
                missing_sections,
            });
        }
    }
    Ok(issues)
}
