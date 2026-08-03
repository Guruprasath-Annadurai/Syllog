//! Adversarial package resolver and persistence tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use semver::Version;
use sha2::{Digest, Sha256};
use syllog_package::{
    CacheError, ContentAddressedCache, InMemoryIndex, PackageRelease, ResolveError, ResolvePolicy,
    lockfile_bytes, resolve, write_lockfile,
};
use syllog_project::{CapabilityProfile, Dependency, Manifest, Package};

fn manifest(dependencies: &[(&str, &str)]) -> Manifest {
    Manifest {
        package: Package {
            name: "app".into(),
            version: "1.0.0".into(),
            edition: "2026".into(),
        },
        targets: Vec::new(),
        dependencies: dependencies
            .iter()
            .map(|(name, requirement)| {
                (
                    (*name).to_owned(),
                    Dependency {
                        requirement: (*requirement).to_owned(),
                    },
                )
            })
            .collect(),
        capabilities: CapabilityProfile {
            profile: "none".into(),
            network: Vec::new(),
            environment: Vec::new(),
            max_memory_bytes: 0,
        },
    }
}

fn release(name: &str, version: &str, dependencies: &[(&str, &str)]) -> PackageRelease {
    let content = format!("{name}-{version}").into_bytes();
    PackageRelease {
        name: name.into(),
        version: Version::parse(version).unwrap(),
        dependencies: dependencies
            .iter()
            .map(|(name, requirement)| ((*name).to_owned(), (*requirement).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        checksum: format!("{:x}", Sha256::digest(&content)),
        content,
        yanked: false,
        available_offline: true,
        archive_paths: vec![PathBuf::from("src/lib.syl")],
    }
}

#[test]
fn exact_and_compatible_requirements_choose_one_deterministic_graph() {
    let index = InMemoryIndex::new(vec![
        release("log", "1.2.0", &[]),
        release("util", "1.5.0", &[("log", "^1.0")]),
        release("util", "1.9.0", &[("log", "^1.2")]),
        release("util", "2.0.0", &[]),
    ]);

    let resolution = resolve(
        &manifest(&[("util", "^1.4"), ("log", "=1.2.0")]),
        &index,
        ResolvePolicy::default(),
    )
    .unwrap();

    assert_eq!(
        resolution
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package.version.to_string()))
            .collect::<Vec<_>>(),
        [("log", "1.2.0".into()), ("util", "1.9.0".into())]
    );
}

#[test]
fn transitive_conflict_reports_all_active_requirements() {
    let index = InMemoryIndex::new(vec![
        release("left", "1.0.0", &[("shared", "^1")]),
        release("right", "1.0.0", &[("shared", "^2")]),
        release("shared", "1.0.0", &[]),
        release("shared", "2.0.0", &[]),
    ]);

    let error = resolve(
        &manifest(&[("left", "=1.0.0"), ("right", "=1.0.0")]),
        &index,
        ResolvePolicy::default(),
    )
    .unwrap_err();

    assert!(matches!(error, ResolveError::Conflict { ref package, .. } if package == "shared"));
    let message = error.to_string();
    assert!(message.contains("left 1.0.0 requires shared ^1"));
    assert!(message.contains("right 1.0.0 requires shared ^2"));
}

#[test]
fn yanked_release_is_rejected_unless_an_exact_requirement_allows_it() {
    let mut yanked = release("old", "1.0.0", &[]);
    yanked.yanked = true;
    let index = InMemoryIndex::new(vec![yanked]);

    assert!(matches!(
        resolve(
            &manifest(&[("old", "^1")]),
            &index,
            ResolvePolicy::default()
        ),
        Err(ResolveError::Conflict { .. })
    ));
    assert!(
        resolve(
            &manifest(&[("old", "=1.0.0")]),
            &index,
            ResolvePolicy::default()
        )
        .is_ok()
    );
}

#[test]
fn checksum_mismatch_is_rejected() {
    let mut corrupt = release("bad", "1.0.0", &[]);
    corrupt.checksum = "0".repeat(64);
    let error = resolve(
        &manifest(&[("bad", "=1.0.0")]),
        &InMemoryIndex::new(vec![corrupt]),
        ResolvePolicy::default(),
    )
    .unwrap_err();
    assert!(matches!(error, ResolveError::ChecksumMismatch { .. }));
}

#[test]
fn offline_mode_rejects_uncached_packages() {
    let mut remote = release("remote", "1.0.0", &[]);
    remote.available_offline = false;
    let error = resolve(
        &manifest(&[("remote", "^1")]),
        &InMemoryIndex::new(vec![remote]),
        ResolvePolicy { offline: true },
    )
    .unwrap_err();
    assert!(matches!(error, ResolveError::OfflineUnavailable { .. }));
}

#[test]
fn archive_path_traversal_is_rejected_before_resolution_is_returned() {
    let mut malicious = release("escape", "1.0.0", &[]);
    malicious.archive_paths = vec![PathBuf::from("src/../../outside")];
    let error = resolve(
        &manifest(&[("escape", "=1.0.0")]),
        &InMemoryIndex::new(vec![malicious]),
        ResolvePolicy::default(),
    )
    .unwrap_err();
    assert!(matches!(error, ResolveError::UnsafeArchivePath { .. }));
}

#[test]
fn shuffled_index_order_produces_byte_identical_lockfile() {
    let releases = vec![
        release("a", "1.0.0", &[("b", "^2")]),
        release("a", "1.1.0", &[("b", "^2")]),
        release("b", "2.0.0", &[]),
        release("b", "2.1.0", &[]),
    ];
    let expected = lockfile_bytes(
        &resolve(
            &manifest(&[("a", "^1")]),
            &InMemoryIndex::new(releases.clone()),
            ResolvePolicy::default(),
        )
        .unwrap(),
    )
    .unwrap();

    for offset in 0..releases.len() {
        let mut shuffled = releases.clone();
        shuffled.rotate_left(offset);
        if offset % 2 == 1 {
            shuffled.reverse();
        }
        let actual = lockfile_bytes(
            &resolve(
                &manifest(&[("a", "^1")]),
                &InMemoryIndex::new(shuffled),
                ResolvePolicy::default(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn solver_backtracks_from_newest_incompatible_release() {
    let index = InMemoryIndex::new(vec![
        release("front", "1.0.0", &[("shared", "^1")]),
        release("front", "1.1.0", &[("shared", "^2")]),
        release("shared", "1.0.0", &[]),
    ]);
    let resolution = resolve(
        &manifest(&[("front", "^1"), ("shared", "^1")]),
        &index,
        ResolvePolicy::default(),
    )
    .unwrap();
    assert_eq!(resolution.packages[0].version, Version::new(1, 0, 0));
}

#[test]
fn lockfile_is_written_atomically_with_canonical_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Syllog.lock");
    let resolution = resolve(
        &manifest(&[("unit", "=1.0.0")]),
        &InMemoryIndex::new(vec![release("unit", "1.0.0", &[])]),
        ResolvePolicy::default(),
    )
    .unwrap();

    write_lockfile(&path, &resolution).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        lockfile_bytes(&resolution).unwrap()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn content_cache_is_immutable_verified_and_path_safe() {
    let directory = tempfile::tempdir().unwrap();
    let cache = ContentAddressedCache::new(directory.path());
    let content = b"verified package";
    let checksum = format!("{:x}", Sha256::digest(content));

    let path = cache.store(&checksum, content).unwrap();
    assert!(path.starts_with(directory.path()));
    assert_eq!(cache.load(&checksum).unwrap(), content);
    assert!(matches!(
        cache.path_for("../../outside"),
        Err(CacheError::InvalidKey(_))
    ));

    std::fs::write(&path, b"tampered").unwrap();
    assert!(matches!(
        cache.load(&checksum),
        Err(CacheError::ChecksumMismatch { .. })
    ));
}
