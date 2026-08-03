//! Executable contracts for Syllog governance and language conformance.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
