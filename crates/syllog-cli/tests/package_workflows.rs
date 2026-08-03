//! End-to-end offline package command contracts.

use std::collections::BTreeMap;
use std::process::Command;

use semver::Version;
use syllog_package::{ContentAddressedCache, LockedPackage, Resolution, write_lockfile};
use syllog_registry_client::{ArchiveFile, PackageArchive};

fn project() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["new", "acme-app", "--template", "basic"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    directory
}

#[test]
fn add_is_atomic_validated_and_preserves_manifest_comments() {
    let directory = project();
    let root = directory.path().join("acme-app");
    let manifest = root.join("Syllog.toml");
    let mut source = std::fs::read_to_string(&manifest).unwrap();
    source.push_str("\n# retain this operator note\n");
    std::fs::write(&manifest, &source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["add", "acme-tools@^1.2"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let edited = std::fs::read_to_string(&manifest).unwrap();
    assert!(edited.contains("# retain this operator note"));
    assert_eq!(
        syllog_project::load_manifest(&manifest)
            .unwrap()
            .dependencies["acme-tools"]
            .requirement,
        "^1.2"
    );

    let before_invalid = edited;
    let invalid = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["add", "../../escape@nope"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), before_invalid);
}

#[test]
fn publish_dry_run_is_deterministic_and_does_not_contact_a_registry() {
    let directory = project();
    let root = directory.path().join("acme-app");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args(["publish", "--dry-run"])
            .current_dir(&root)
            .output()
            .unwrap()
    };
    let first = run();
    let second = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["name"], "acme-app");
    assert_eq!(
        report["files"],
        serde_json::json!(["Syllog.toml", "src/main.syl"])
    );
}

#[test]
fn vendor_rebuilds_locked_packages_from_verified_offline_cache() {
    let directory = project();
    let root = directory.path().join("acme-app");
    let archive = PackageArchive::new(
        "acme-tools",
        Version::new(1, 2, 3),
        vec![ArchiveFile::new(
            "src/lib.syl",
            b"pub fn tool() -> U64 { 7 }".to_vec(),
        )],
        BTreeMap::new(),
    )
    .unwrap();
    let bytes = archive.canonical_bytes().unwrap();
    let checksum = archive.checksum().unwrap();
    ContentAddressedCache::new(root.join(".syllog/cache"))
        .store(&checksum, &bytes)
        .unwrap();
    write_lockfile(
        &root.join("Syllog.lock"),
        &Resolution {
            format: 1,
            packages: vec![LockedPackage {
                name: "acme-tools".into(),
                version: Version::new(1, 2, 3),
                checksum,
                dependencies: BTreeMap::new(),
            }],
        },
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .arg("vendor")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("vendor/acme-tools-1.2.3/src/lib.syl")).unwrap(),
        "pub fn tool() -> U64 { 7 }"
    );
}
