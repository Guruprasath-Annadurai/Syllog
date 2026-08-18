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

const CANONICAL_REPOSITORY_URL: &str = "https://github.com/Guruprasath-Annadurai/Syllog";

const REQUIRED_ROOT_DOCUMENTS: [&str; 5] = [
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    "ROADMAP.md",
    "SECURITY.md",
    "docs/supported-platforms.md",
];

/// One executable language conformance case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceCase {
    /// Language edition used to compile the source.
    pub edition: String,
    /// Normalized absolute source path.
    pub source: PathBuf,
    /// Normative rule identifiers exercised by this source.
    pub rules: Vec<String>,
    /// Whether this case demonstrates acceptance or mandatory rejection.
    pub polarity: CasePolarity,
    /// Required compiler or runtime outcome.
    pub expected: ExpectedOutcome,
}

/// The acceptance direction covered by a conformance fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CasePolarity {
    /// A valid program accepted by the implementation.
    Positive,
    /// An invalid program rejected with stable diagnostics.
    Negative,
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
    rules: Vec<String>,
    polarity: CasePolarity,
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
        cases.push(load_case(&root, case, &mut identities)?);
    }
    cases.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.edition.cmp(&right.edition))
    });
    Ok(cases)
}

fn load_case(
    root: &Path,
    case: ManifestCase,
    identities: &mut BTreeSet<(String, PathBuf)>,
) -> anyhow::Result<ConformanceCase> {
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
    validate_case_rules(&source, &case.rules)?;
    let expected = load_expected(case.expected)?;
    validate_polarity(&source, case.polarity, &expected)?;
    Ok(ConformanceCase {
        edition: case.edition,
        source,
        rules: case.rules,
        polarity: case.polarity,
        expected,
    })
}

fn validate_case_rules(source: &Path, rules: &[String]) -> anyhow::Result<()> {
    if rules.is_empty() {
        bail!(
            "conformance case {} must name at least one rule",
            source.display()
        );
    }
    let mut unique_rules = BTreeSet::new();
    for rule in rules {
        if !valid_rule_id(rule) {
            bail!("invalid normative rule identifier '{rule}'");
        }
        if !unique_rules.insert(rule) {
            bail!(
                "duplicate normative rule identifier '{rule}' in {}",
                source.display()
            );
        }
    }
    Ok(())
}

fn load_expected(expected: ManifestOutcome) -> anyhow::Result<ExpectedOutcome> {
    Ok(match expected {
        ManifestOutcome::Pass => ExpectedOutcome::Pass,
        ManifestOutcome::Diagnostics { codes } => {
            if codes.is_empty() || codes.iter().any(|code| code.trim().is_empty()) {
                bail!("diagnostic conformance cases require non-empty codes");
            }
            ExpectedOutcome::Diagnostics(codes)
        }
        ManifestOutcome::Run { stdout, exit_code } => ExpectedOutcome::Run { stdout, exit_code },
    })
}

fn validate_polarity(
    source: &Path,
    polarity: CasePolarity,
    expected: &ExpectedOutcome,
) -> anyhow::Result<()> {
    match (polarity, expected) {
        (CasePolarity::Positive, ExpectedOutcome::Diagnostics(_)) => {
            bail!(
                "positive conformance case {} cannot expect diagnostics",
                source.display()
            );
        }
        (CasePolarity::Negative, ExpectedOutcome::Pass | ExpectedOutcome::Run { .. }) => {
            bail!(
                "negative conformance case {} must expect diagnostics",
                source.display()
            );
        }
        _ => Ok(()),
    }
}

/// A normative rule that lacks executable acceptance or rejection coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleCoverageGap {
    /// Stable identifier from the language reference.
    pub rule_id: String,
    /// No positive fixture names this rule.
    pub missing_positive: bool,
    /// No negative fixture names this rule.
    pub missing_negative: bool,
}

/// Loads stable normative rule identifiers from the implemented-rule table in
/// the language reference. Rule identifiers use the `SYL-AREA-NAME-NNN` form.
///
/// # Errors
///
/// Returns an error when the document is unreadable, contains a duplicate or
/// malformed identifier, or defines no executable normative rules.
pub fn load_normative_rule_ids(path: &Path) -> anyhow::Result<Vec<String>> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("could not read language reference {}", path.display()))?;
    let mut rules = BTreeSet::new();
    for line in contents.lines() {
        let Some(start) = line.find("`SYL-") else {
            continue;
        };
        let identifier = &line[start + 1..];
        let Some(end) = identifier.find('`') else {
            bail!(
                "unterminated normative rule identifier in {}",
                path.display()
            );
        };
        let identifier = &identifier[..end];
        if !valid_rule_id(identifier) {
            bail!("invalid normative rule identifier '{identifier}'");
        }
        if !rules.insert(identifier.to_owned()) {
            bail!("duplicate normative rule identifier '{identifier}'");
        }
    }
    if rules.is_empty() {
        bail!("language reference defines no executable normative rule identifiers");
    }
    Ok(rules.into_iter().collect())
}

/// Reports missing positive and negative fixture coverage for every normative
/// rule. References to identifiers absent from the language reference are also
/// returned as gaps so stale manifests cannot silently pass.
#[must_use]
pub fn validate_rule_coverage(
    normative_rules: &[String],
    cases: &[ConformanceCase],
) -> Vec<RuleCoverageGap> {
    let normative: BTreeSet<_> = normative_rules.iter().cloned().collect();
    let mut positive = BTreeSet::new();
    let mut negative = BTreeSet::new();
    for case in cases {
        let destination = match case.polarity {
            CasePolarity::Positive => &mut positive,
            CasePolarity::Negative => &mut negative,
        };
        destination.extend(case.rules.iter().cloned());
    }

    let referenced: BTreeSet<_> = positive.union(&negative).cloned().collect();
    normative
        .union(&referenced)
        .filter_map(|rule_id| {
            let missing_positive = !positive.contains(rule_id);
            let missing_negative = !negative.contains(rule_id);
            if !normative.contains(rule_id) || missing_positive || missing_negative {
                Some(RuleCoverageGap {
                    rule_id: rule_id.clone(),
                    missing_positive,
                    missing_negative,
                })
            } else {
                None
            }
        })
        .collect()
}

fn valid_rule_id(identifier: &str) -> bool {
    identifier.starts_with("SYL-")
        && identifier.len() >= "SYL-A-B-000".len()
        && identifier.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '-'
        })
        && identifier.rsplit('-').next().is_some_and(|suffix| {
            suffix.len() == 3 && suffix.chars().all(|digit| digit.is_ascii_digit())
        })
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

/// One repository identity, automation, or documentation truth violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryTruthIssue {
    /// Stable identifier suitable for CI output and regression tests.
    pub code: &'static str,
    /// Repository-relative path containing the violation.
    pub path: PathBuf,
    /// Human-readable explanation of the failed contract.
    pub message: String,
}

/// Validates the non-negotiable repository-truth contracts for Layer 0.
///
/// This intentionally checks stable facts rather than parsing every file format:
/// the canonical repository identity, default-branch CI, supported CI hosts,
/// required community documents, and declarations of the authoritative parser
/// and compiler pipeline.
///
/// # Errors
///
/// Returns an I/O error when a required input exists but cannot be read.
pub fn validate_repository_truth(repository: &Path) -> io::Result<Vec<RepositoryTruthIssue>> {
    let mut issues = Vec::new();

    check_contains(
        repository,
        "Cargo.toml",
        CANONICAL_REPOSITORY_URL,
        "repository.identity.cargo",
        &mut issues,
    )?;
    check_contains(
        repository,
        "README.md",
        CANONICAL_REPOSITORY_URL,
        "repository.identity.readme",
        &mut issues,
    )?;

    for document in REQUIRED_ROOT_DOCUMENTS {
        if !repository.join(document).is_file() {
            issues.push(RepositoryTruthIssue {
                code: "repository.document.missing",
                path: PathBuf::from(document),
                message: "required Layer 0 document is missing".into(),
            });
        }
    }

    let workflow = ".github/workflows/ci.yml";
    for (needle, code) in [
        ("branches: [main]", "repository.ci.default_branch"),
        ("pull_request:", "repository.ci.pull_request"),
        ("ubuntu-latest", "repository.ci.linux"),
        ("macos-latest", "repository.ci.macos"),
        ("windows-latest", "repository.ci.windows"),
    ] {
        check_contains(repository, workflow, needle, code, &mut issues)?;
    }

    for (path, needle, code) in [
        (
            "docs/design.md",
            "crates/syllog-parser/src/grammar.pest",
            "repository.authority.parser",
        ),
        (
            "docs/design.md",
            "crates/syllog-compiler",
            "repository.authority.compiler",
        ),
        (
            "docs/feature-status.md",
            "## Implemented",
            "repository.status.matrix",
        ),
        (
            "docs/required-branch-checks.md",
            "workspace (ubuntu-latest)",
            "repository.ci.required_checks",
        ),
    ] {
        check_contains(repository, path, needle, code, &mut issues)?;
    }

    Ok(issues)
}

fn check_contains(
    repository: &Path,
    relative: &str,
    needle: &str,
    code: &'static str,
    issues: &mut Vec<RepositoryTruthIssue>,
) -> io::Result<()> {
    let path = repository.join(relative);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            issues.push(RepositoryTruthIssue {
                code,
                path: PathBuf::from(relative),
                message: "required file is missing".into(),
            });
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if !contents.contains(needle) {
        issues.push(RepositoryTruthIssue {
            code,
            path: PathBuf::from(relative),
            message: format!("required content is absent: {needle}"),
        });
    }
    Ok(())
}
