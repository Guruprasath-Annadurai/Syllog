//! Deterministic semantic-version package resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, PathBuf};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use syllog_project::Manifest;
use thiserror::Error;

/// Immutable metadata and verified archive bytes for one package release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageRelease {
    /// Registry package name.
    pub name: String,
    /// Published semantic version.
    pub version: Version,
    /// Direct package requirements.
    pub dependencies: BTreeMap<String, String>,
    /// Expected lowercase SHA-256 digest of `content`.
    pub checksum: String,
    /// Canonical package archive bytes.
    pub content: Vec<u8>,
    /// Whether the registry has withdrawn this version from new resolutions.
    pub yanked: bool,
    /// Whether the content-addressed archive is locally available.
    pub available_offline: bool,
    /// Paths declared by the archive directory.
    pub archive_paths: Vec<PathBuf>,
}

/// Read-only source of package release metadata and content.
pub trait PackageIndex {
    /// Returns every known release for `name`; ordering is not significant.
    fn releases(&self, name: &str) -> Vec<PackageRelease>;
}

/// Deterministic index used by tests, offline stores, and embedded registries.
#[derive(Clone, Debug, Default)]
pub struct InMemoryIndex {
    releases: BTreeMap<String, Vec<PackageRelease>>,
}

impl InMemoryIndex {
    /// Builds an index. Duplicate name/version entries remain visible to the
    /// resolver and are rejected unless they are byte-for-byte identical.
    #[must_use]
    pub fn new(releases: Vec<PackageRelease>) -> Self {
        let mut grouped = BTreeMap::<String, Vec<PackageRelease>>::new();
        for release in releases {
            grouped
                .entry(release.name.clone())
                .or_default()
                .push(release);
        }
        Self { releases: grouped }
    }
}

impl PackageIndex for InMemoryIndex {
    fn releases(&self, name: &str) -> Vec<PackageRelease> {
        self.releases.get(name).cloned().unwrap_or_default()
    }
}

/// Resolution behavior that can affect package availability.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResolvePolicy {
    /// Forbid releases whose archives are not present locally.
    pub offline: bool,
}

/// One immutable package entry selected for a lockfile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LockedPackage {
    /// Registry package name.
    pub name: String,
    /// Selected semantic version.
    pub version: Version,
    /// Verified SHA-256 content digest.
    pub checksum: String,
    /// Exact versions of direct dependencies.
    pub dependencies: BTreeMap<String, Version>,
}

/// Complete deterministic solution for a root manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Resolution {
    /// Lockfile format version.
    pub format: u32,
    /// Packages sorted by name and then version.
    pub packages: Vec<LockedPackage>,
}

/// Stable package resolution failure.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ResolveError {
    /// A requirement is not valid `SemVer` syntax.
    #[error("invalid version requirement '{requirement}' for package '{package}': {reason}")]
    InvalidRequirement {
        /// Affected package.
        package: String,
        /// Invalid text.
        requirement: String,
        /// Parser explanation.
        reason: String,
    },
    /// No release satisfies all active constraints.
    #[error("cannot resolve package '{package}': {}", requirements.join("; "))]
    Conflict {
        /// Affected package.
        package: String,
        /// Deterministically ordered active requirements and their origins.
        requirements: Vec<String>,
    },
    /// An archive needed by an offline resolution is absent.
    #[error("package '{package}' version {version} is unavailable offline")]
    OfflineUnavailable {
        /// Affected package.
        package: String,
        /// Best otherwise-compatible version.
        version: Version,
    },
    /// Registry metadata did not match downloaded content.
    #[error(
        "checksum mismatch for package '{package}' version {version}: expected {expected}, computed {actual}"
    )]
    ChecksumMismatch {
        /// Affected package.
        package: String,
        /// Affected version.
        version: Version,
        /// Registry digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },
    /// An archive entry could escape its extraction root.
    #[error("unsafe archive path '{}' in package '{package}' version {version}", path.display())]
    UnsafeArchivePath {
        /// Affected package.
        package: String,
        /// Affected version.
        version: Version,
        /// Rejected path.
        path: PathBuf,
    },
    /// One name/version maps to conflicting registry records.
    #[error("registry contains conflicting records for package '{package}' version {version}")]
    AmbiguousRelease {
        /// Affected package.
        package: String,
        /// Affected version.
        version: Version,
    },
}

#[derive(Clone, Debug)]
struct Constraint {
    requirement: VersionReq,
    text: String,
    origin: String,
    exact: bool,
}

/// Resolves a manifest into a reproducible, verified package graph.
///
/// The solver explores versions in descending order and package names in
/// lexical order. It backtracks across transitive incompatibilities, while all
/// externally observable collections are sorted.
///
/// # Errors
///
/// Returns a stable error for invalid constraints, an unsatisfiable graph,
/// unavailable offline content, corrupt content, unsafe archive paths, or
/// ambiguous registry records.
pub fn resolve(
    manifest: &Manifest,
    index: &dyn PackageIndex,
    policy: ResolvePolicy,
) -> Result<Resolution, ResolveError> {
    let mut constraints = BTreeMap::<String, Vec<Constraint>>::new();
    for (name, dependency) in &manifest.dependencies {
        add_constraint(
            &mut constraints,
            name,
            &dependency.requirement,
            format!("root requires {name} {}", dependency.requirement),
        )?;
    }

    let selected = search(index, policy, BTreeMap::new(), &constraints)?;
    let packages = selected
        .values()
        .map(|release| LockedPackage {
            name: release.name.clone(),
            version: release.version.clone(),
            checksum: release.checksum.clone(),
            dependencies: release
                .dependencies
                .keys()
                .filter_map(|name| {
                    selected
                        .get(name)
                        .map(|dependency| (name.clone(), dependency.version.clone()))
                })
                .collect(),
        })
        .collect();
    Ok(Resolution {
        format: 1,
        packages,
    })
}

fn search(
    index: &dyn PackageIndex,
    policy: ResolvePolicy,
    selected: BTreeMap<String, PackageRelease>,
    constraints: &BTreeMap<String, Vec<Constraint>>,
) -> Result<BTreeMap<String, PackageRelease>, ResolveError> {
    for (name, release) in &selected {
        if let Some(active) = constraints.get(name)
            && !active
                .iter()
                .all(|constraint| constraint.requirement.matches(&release.version))
        {
            return Err(conflict(name, active));
        }
    }

    let Some(name) = constraints
        .keys()
        .find(|name| !selected.contains_key(*name))
        .cloned()
    else {
        return Ok(selected);
    };
    let active = &constraints[&name];
    let mut releases = index.releases(&name);
    releases.sort_by(|left, right| {
        right
            .version
            .cmp(&left.version)
            .then_with(|| left.checksum.cmp(&right.checksum))
    });
    reject_ambiguous(&name, &releases)?;

    let mut compatible = releases
        .into_iter()
        .filter(|release| {
            active
                .iter()
                .all(|constraint| constraint.requirement.matches(&release.version))
                && (!release.yanked
                    || active.iter().any(|constraint| {
                        constraint.exact && constraint.requirement.matches(&release.version)
                    }))
        })
        .collect::<Vec<_>>();

    if policy.offline {
        if let Some(unavailable) = compatible.iter().find(|release| !release.available_offline)
            && compatible.iter().all(|release| !release.available_offline)
        {
            return Err(ResolveError::OfflineUnavailable {
                package: name,
                version: unavailable.version.clone(),
            });
        }
        compatible.retain(|release| release.available_offline);
    }
    if compatible.is_empty() {
        return Err(conflict(&name, active));
    }

    let mut last_conflict = None;
    for release in compatible {
        validate_release(&release)?;
        let mut next_selected = selected.clone();
        next_selected.insert(name.clone(), release.clone());
        let mut next_constraints = constraints.clone();
        for (dependency, requirement) in &release.dependencies {
            add_constraint(
                &mut next_constraints,
                dependency,
                requirement,
                format!(
                    "{} {} requires {dependency} {requirement}",
                    release.name, release.version
                ),
            )?;
        }
        match search(index, policy, next_selected, &next_constraints) {
            Ok(solution) => return Ok(solution),
            Err(error @ ResolveError::Conflict { .. }) => last_conflict = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_conflict.unwrap_or_else(|| conflict(&name, active)))
}

fn add_constraint(
    constraints: &mut BTreeMap<String, Vec<Constraint>>,
    package: &str,
    text: &str,
    origin: String,
) -> Result<(), ResolveError> {
    let requirement =
        VersionReq::parse(text).map_err(|error| ResolveError::InvalidRequirement {
            package: package.to_owned(),
            requirement: text.to_owned(),
            reason: error.to_string(),
        })?;
    constraints
        .entry(package.to_owned())
        .or_default()
        .push(Constraint {
            requirement,
            text: text.to_owned(),
            origin,
            exact: text.trim_start().starts_with('='),
        });
    Ok(())
}

fn conflict(package: &str, constraints: &[Constraint]) -> ResolveError {
    let requirements = constraints
        .iter()
        .map(|constraint| format!("{} ({})", constraint.origin, constraint.text))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ResolveError::Conflict {
        package: package.to_owned(),
        requirements,
    }
}

fn reject_ambiguous(name: &str, releases: &[PackageRelease]) -> Result<(), ResolveError> {
    for pair in releases.windows(2) {
        if pair[0].version == pair[1].version && pair[0] != pair[1] {
            return Err(ResolveError::AmbiguousRelease {
                package: name.to_owned(),
                version: pair[0].version.clone(),
            });
        }
    }
    Ok(())
}

fn validate_release(release: &PackageRelease) -> Result<(), ResolveError> {
    let actual = format!("{:x}", Sha256::digest(&release.content));
    if actual != release.checksum {
        return Err(ResolveError::ChecksumMismatch {
            package: release.name.clone(),
            version: release.version.clone(),
            expected: release.checksum.clone(),
            actual,
        });
    }
    for path in &release.archive_paths {
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ResolveError::UnsafeArchivePath {
                package: release.name.clone(),
                version: release.version.clone(),
                path: path.clone(),
            });
        }
    }
    Ok(())
}
