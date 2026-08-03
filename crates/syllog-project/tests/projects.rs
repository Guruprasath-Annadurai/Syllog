//! Project discovery and strict manifest contracts.

use std::path::Path;

use syllog_project::{TargetKind, discover, load_manifest, manifest_schema};

const VALID_MANIFEST: &str = r#"
[package]
name = "frontier-agent"
version = "0.1.0"
edition = "2026"

[[targets]]
name = "agent"
kind = "bin"
path = "src/./main.syl"

[dependencies]
telemetry = "1.2.3"

[capabilities]
profile = "agent"
network = ["api.example.com:443"]
environment = ["MODEL_API_KEY"]
max_memory_bytes = 67108864
"#;

#[test]
fn discovers_parent_manifest_and_normalizes_target_paths() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    std::fs::create_dir_all(directory.path().join("src/nested"))
        .expect("nested source directory should exist");
    std::fs::write(directory.path().join("Syllog.toml"), VALID_MANIFEST)
        .expect("manifest should be written");

    let project = discover(&directory.path().join("src/nested"))
        .expect("discovery should walk to the parent manifest");

    assert_eq!(
        project.root,
        directory
            .path()
            .canonicalize()
            .expect("temporary root should canonicalize")
    );
    assert_eq!(project.manifest.package.name, "frontier-agent");
    assert_eq!(project.manifest.targets[0].kind, TargetKind::Bin);
    assert_eq!(
        project.manifest.targets[0].path,
        project.root.join("src/main.syl")
    );
    assert_eq!(
        project.manifest.capabilities.network,
        ["api.example.com:443"]
    );
    assert_eq!(
        project.manifest.dependencies["telemetry"].requirement,
        "1.2.3"
    );
}

#[test]
fn unknown_keys_produce_stable_spanned_json_diagnostics() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    let path = directory.path().join("Syllog.toml");
    std::fs::write(&path, format!("{VALID_MANIFEST}\nexperimental = true\n"))
        .expect("manifest should be written");

    let diagnostics = load_manifest(&path).expect_err("unknown keys must be rejected");

    assert_eq!(diagnostics[0].code, "SYLP1001");
    assert_eq!(diagnostics[0].file, path);
    assert!(diagnostics[0].message.contains("experimental"));
    assert!(diagnostics[0].range.start.line > 0);
    assert!(serde_json::to_value(&diagnostics).is_ok());
}

#[test]
fn duplicate_targets_are_rejected_at_the_second_name() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    let path = directory.path().join("Syllog.toml");
    let duplicate = VALID_MANIFEST.replace(
        "[dependencies]",
        "[[targets]]\nname = \"agent\"\nkind = \"lib\"\npath = \"src/lib.syl\"\n\n[dependencies]",
    );
    std::fs::write(&path, duplicate).expect("manifest should be written");

    let diagnostics = load_manifest(&path).expect_err("duplicate targets must be rejected");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "SYLP1002");
    assert!(diagnostics[0].message.contains("agent"));
    assert!(diagnostics[0].range.start.line > 10);
}

#[test]
fn capability_profiles_are_strict_and_paths_cannot_escape_project() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    let path = directory.path().join("Syllog.toml");
    let invalid = VALID_MANIFEST
        .replace("profile = \"agent\"", "profile = \"unbounded\"")
        .replace("src/./main.syl", "../main.syl");
    std::fs::write(&path, invalid).expect("manifest should be written");

    let diagnostics = load_manifest(&path).expect_err("unsafe manifest must be rejected");
    let codes = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();

    assert_eq!(codes, ["SYLP1003", "SYLP1004"]);
}

#[test]
fn schema_is_versioned_and_rejects_unknown_properties() {
    let schema = manifest_schema();

    assert_eq!(schema["$id"], "https://syllog.dev/schema/manifest-1.json");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["package"]["type"], "object");
}

#[test]
fn discovery_reports_the_start_path_when_no_manifest_exists() {
    let directory = tempfile::tempdir().expect("temporary directory should exist");
    let error = discover(directory.path()).expect_err("missing manifest must be reported");

    assert!(
        error
            .to_string()
            .contains(Path::new(directory.path()).to_str().unwrap())
    );
}
