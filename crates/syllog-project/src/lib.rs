//! Syllog project manifest loading and discovery.

mod discover;
mod manifest;

pub use discover::{Project, ProjectError, discover};
pub use manifest::{
    CapabilityProfile, Dependency, DependencyMap, Manifest, ManifestDiagnostic, Package,
    SourcePosition, SourceRange, Target, TargetKind, load_manifest, manifest_schema,
};
