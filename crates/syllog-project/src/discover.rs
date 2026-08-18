//! Parent-directory project discovery.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{Manifest, ManifestDiagnostic, load_manifest};

/// A discovered project root and validated manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    /// Directory containing `Syllog.toml`.
    pub root: PathBuf,
    /// Absolute manifest path.
    pub manifest_path: PathBuf,
    /// Validated manifest contents.
    pub manifest: Manifest,
}

/// Project discovery failure.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// The start path could not be resolved.
    #[error("could not resolve project search path {path}: {source}")]
    StartPath {
        /// User-provided search path.
        path: PathBuf,
        /// Filesystem error.
        source: std::io::Error,
    },
    /// No parent contained a manifest.
    #[error("no Syllog.toml found from {start} or its parents")]
    NotFound {
        /// Resolved starting directory.
        start: PathBuf,
    },
    /// A discovered manifest was invalid.
    #[error("invalid Syllog.toml: {diagnostics:?}")]
    InvalidManifest {
        /// Stable validation diagnostics.
        diagnostics: Vec<ManifestDiagnostic>,
    },
}

/// Discovers the nearest project by walking from `start` toward the root.
///
/// # Errors
///
/// Returns an error when the start path is inaccessible, no manifest exists,
/// or the nearest manifest is invalid.
pub fn discover(start: &Path) -> Result<Project, ProjectError> {
    let reported_start = if start.is_file() {
        start.parent().unwrap_or(start).to_owned()
    } else {
        start.to_owned()
    };
    let resolved = start
        .canonicalize()
        .map_err(|source| ProjectError::StartPath {
            path: start.to_owned(),
            source,
        })?;
    let start_directory = if resolved.is_file() {
        resolved.parent().unwrap_or(&resolved).to_owned()
    } else {
        resolved
    };
    for directory in start_directory.ancestors() {
        let manifest_path = directory.join("Syllog.toml");
        if manifest_path.is_file() {
            let manifest = load_manifest(&manifest_path)
                .map_err(|diagnostics| ProjectError::InvalidManifest { diagnostics })?;
            return Ok(Project {
                root: directory.to_owned(),
                manifest_path,
                manifest,
            });
        }
    }
    Err(ProjectError::NotFound {
        // Canonical Windows paths can acquire a `\\?\` prefix or resolve
        // through a junction. Diagnostics should preserve the path the user
        // supplied while discovery itself continues to use the canonical path.
        start: reported_start,
    })
}
