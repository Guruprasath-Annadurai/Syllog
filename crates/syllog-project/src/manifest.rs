//! Strict, versioned `Syllog.toml` representation.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A validated project manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Manifest {
    /// Package identity and language edition.
    pub package: Package,
    /// Executable and library targets in declaration order.
    pub targets: Vec<Target>,
    /// Reproducible package requirements, ordered by package name.
    pub dependencies: DependencyMap,
    /// Runtime authority requested by this package.
    pub capabilities: CapabilityProfile,
}

/// Package identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Package {
    /// Registry-safe package name.
    pub name: String,
    /// Semantic package version.
    pub version: String,
    /// Language edition selected by the package.
    pub edition: String,
}

/// A buildable project target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Target {
    /// Target-local name.
    pub name: String,
    /// Target output kind.
    pub kind: TargetKind,
    /// Absolute, lexically normalized source path.
    pub path: PathBuf,
}

/// Supported target output kinds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    /// Executable target.
    Bin,
    /// Reusable library target.
    Lib,
}

/// Ordered dependency requirements.
pub type DependencyMap = BTreeMap<String, Dependency>;

/// One registry dependency.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Dependency {
    /// Exact or compatible version requirement text.
    pub requirement: String,
}

/// Runtime authority profile requested by a target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CapabilityProfile {
    /// Named conservative baseline (`none`, `agent`, or `native`).
    pub profile: String,
    /// Explicit network endpoints.
    pub network: Vec<String>,
    /// Environment variable names visible to the program.
    pub environment: Vec<String>,
    /// Maximum linear memory granted to the runtime.
    pub max_memory_bytes: u64,
}

/// One-based source position suitable for editors and JSON protocols.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SourcePosition {
    /// One-based line.
    pub line: usize,
    /// One-based Unicode-scalar column.
    pub column: usize,
}

/// Half-open source range.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SourceRange {
    /// First covered position.
    pub start: SourcePosition,
    /// Position immediately after the range.
    pub end: SourcePosition,
}

/// Stable project diagnostic serializable by editors and CI systems.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManifestDiagnostic {
    /// Stable diagnostic identifier.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Manifest file containing the fault.
    pub file: PathBuf,
    /// Exact source range when available.
    pub range: SourceRange,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    package: RawPackage,
    #[serde(default)]
    targets: Vec<RawTarget>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    capabilities: RawCapabilities,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    name: String,
    version: String,
    edition: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    name: toml::Spanned<String>,
    kind: TargetKind,
    path: toml::Spanned<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilities {
    profile: toml::Spanned<String>,
    #[serde(default)]
    network: Vec<String>,
    #[serde(default)]
    environment: Vec<String>,
    max_memory_bytes: u64,
}

/// Loads and validates a strict `Syllog.toml` manifest.
///
/// # Errors
///
/// Returns stable, source-ranged diagnostics for syntax, schema, or semantic
/// manifest faults.
pub fn load_manifest(path: &Path) -> Result<Manifest, Vec<ManifestDiagnostic>> {
    let source = std::fs::read_to_string(path).map_err(|error| {
        vec![diagnostic(
            "SYLP1000",
            format!("could not read manifest: {error}"),
            path,
            "",
            0..0,
        )]
    })?;
    let raw: RawManifest = toml::from_str(&source).map_err(|error| {
        vec![diagnostic(
            "SYLP1001",
            error.message().to_owned(),
            path,
            &source,
            error.span().unwrap_or(0..0),
        )]
    })?;
    validate(path, &source, raw)
}

fn validate(
    path: &Path,
    source: &str,
    raw: RawManifest,
) -> Result<Manifest, Vec<ManifestDiagnostic>> {
    let mut diagnostics = Vec::new();
    let profile_span = raw.capabilities.profile.span();
    let profile = raw.capabilities.profile.into_inner();
    if !matches!(profile.as_str(), "none" | "agent" | "native") {
        diagnostics.push(diagnostic(
            "SYLP1003",
            format!("unknown capability profile '{profile}'"),
            path,
            source,
            profile_span,
        ));
    }

    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let mut names = BTreeSet::new();
    let mut targets = Vec::with_capacity(raw.targets.len());
    for target in raw.targets {
        let name_span = target.name.span();
        let name = target.name.into_inner();
        if !names.insert(name.clone()) {
            diagnostics.push(diagnostic(
                "SYLP1002",
                format!("duplicate target '{name}'"),
                path,
                source,
                name_span,
            ));
        }
        let target_span = target.path.span();
        let relative = target.path.into_inner();
        match normalize_inside(root, &relative) {
            Some(normalized) => targets.push(Target {
                name,
                kind: target.kind,
                path: normalized,
            }),
            None => diagnostics.push(diagnostic(
                "SYLP1004",
                format!(
                    "target path '{}' escapes the project root",
                    relative.display()
                ),
                path,
                source,
                target_span,
            )),
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(Manifest {
        package: Package {
            name: raw.package.name,
            version: raw.package.version,
            edition: raw.package.edition,
        },
        targets,
        dependencies: raw
            .dependencies
            .into_iter()
            .map(|(name, requirement)| (name, Dependency { requirement }))
            .collect(),
        capabilities: CapabilityProfile {
            profile,
            network: raw.capabilities.network,
            environment: raw.capabilities.environment,
            max_memory_bytes: raw.capabilities.max_memory_bytes,
        },
    })
}

fn normalize_inside(root: &Path, relative: &Path) -> Option<PathBuf> {
    if relative.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir if !normalized.pop() => return None,
            Component::CurDir | Component::ParentDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(root.join(normalized))
}

fn diagnostic(
    code: &str,
    message: String,
    file: &Path,
    source: &str,
    span: Range<usize>,
) -> ManifestDiagnostic {
    ManifestDiagnostic {
        code: code.to_owned(),
        message,
        file: file.to_owned(),
        range: SourceRange {
            start: position(source, span.start),
            end: position(source, span.end),
        },
    }
}

fn position(source: &str, byte: usize) -> SourcePosition {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    SourcePosition { line, column }
}

/// Returns the versioned JSON Schema used for editor manifest validation.
#[must_use]
pub fn manifest_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://syllog.dev/schema/manifest-1.json",
        "title": "Syllog project manifest",
        "type": "object",
        "additionalProperties": false,
        "required": ["package", "targets", "capabilities"],
        "properties": {
            "package": { "type": "object" },
            "targets": { "type": "array" },
            "dependencies": { "type": "object" },
            "capabilities": { "type": "object" }
        }
    })
}
