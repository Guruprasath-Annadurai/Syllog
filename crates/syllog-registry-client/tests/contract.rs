//! Offline local-registry security contract.

use std::collections::BTreeMap;

use semver::Version;
use syllog_registry_client::{
    ArchiveFile, LocalRegistry, PackageArchive, PublisherIdentity, RegistryError,
};

fn archive(name: &str, version: &str) -> PackageArchive {
    PackageArchive::new(
        name,
        Version::parse(version).unwrap(),
        vec![
            ArchiveFile::new("Syllog.toml", b"[package]".to_vec()),
            ArchiveFile::new("src/lib.syl", b"pub fn value() -> U64 { 1 }".to_vec()),
        ],
        BTreeMap::new(),
    )
    .unwrap()
}

fn identity() -> PublisherIdentity {
    PublisherIdentity::from_seed("acme", [7; 32])
}

fn registry(identity: &PublisherIdentity) -> LocalRegistry {
    let mut registry = LocalRegistry::new();
    registry.authorize("acme", identity.verifying_key());
    registry
}

#[test]
fn signed_publication_round_trips_verified_content() {
    let identity = identity();
    let mut registry = registry(&identity);
    let package = archive("acme-tools", "1.0.0");
    let request = identity.sign(package.clone(), "nonce-1").unwrap();

    let receipt = registry.publish(request).unwrap();
    let downloaded = registry
        .download("acme-tools", &Version::new(1, 0, 0))
        .unwrap();

    assert_eq!(downloaded, package);
    assert_eq!(receipt.checksum, package.checksum().unwrap());
}

#[test]
fn versions_are_immutable_and_publish_nonces_cannot_be_replayed() {
    let identity = identity();
    let mut registry = registry(&identity);
    registry
        .publish(
            identity
                .sign(archive("acme-tools", "1.0.0"), "nonce-1")
                .unwrap(),
        )
        .unwrap();

    let immutable = identity
        .sign(archive("acme-tools", "1.0.0"), "nonce-2")
        .unwrap();
    assert!(matches!(
        registry.publish(immutable),
        Err(RegistryError::ImmutableVersion { .. })
    ));

    let replay = identity
        .sign(archive("acme-other", "1.0.0"), "nonce-1")
        .unwrap();
    assert!(matches!(
        registry.publish(replay),
        Err(RegistryError::Replay { .. })
    ));
}

#[test]
fn namespace_authorization_and_signature_are_enforced() {
    let identity = identity();
    let mut registry = registry(&identity);
    let intruder = PublisherIdentity::from_seed("acme", [9; 32]);
    assert!(matches!(
        registry.publish(
            intruder
                .sign(archive("acme-tools", "1.0.0"), "nonce-x")
                .unwrap()
        ),
        Err(RegistryError::Unauthorized { .. })
    ));
    assert!(matches!(
        registry.publish(
            identity
                .sign(archive("other-tools", "1.0.0"), "nonce-y")
                .unwrap()
        ),
        Err(RegistryError::NamespaceMismatch { .. })
    ));
}

#[test]
fn checksum_tampering_is_rejected_before_storage() {
    let identity = identity();
    let mut registry = registry(&identity);
    let mut request = identity
        .sign(archive("acme-tools", "1.0.0"), "nonce-1")
        .unwrap();
    request.archive.files[0].content.push(0);
    assert!(matches!(
        registry.publish(request),
        Err(RegistryError::ChecksumMismatch { .. })
    ));
}

#[test]
fn signature_tampering_is_rejected_before_storage() {
    let identity = identity();
    let mut registry = registry(&identity);
    let mut request = identity
        .sign(archive("acme-tools", "1.0.0"), "nonce-1")
        .unwrap();
    request.signature[0] ^= 1;
    assert!(matches!(
        registry.publish(request),
        Err(RegistryError::InvalidSignature { .. })
    ));
}

#[test]
fn unsafe_archive_paths_are_rejected_at_construction() {
    assert!(matches!(
        PackageArchive::new(
            "acme-escape",
            Version::new(1, 0, 0),
            vec![ArchiveFile::new("src/../../escape", vec![])],
            BTreeMap::new()
        ),
        Err(RegistryError::UnsafeArchivePath { .. })
    ));
}

#[test]
fn yanked_versions_remain_downloadable_but_are_marked_for_resolvers() {
    let identity = identity();
    let mut registry = registry(&identity);
    registry
        .publish(
            identity
                .sign(archive("acme-tools", "1.0.0"), "nonce-1")
                .unwrap(),
        )
        .unwrap();
    registry.yank("acme-tools", &Version::new(1, 0, 0)).unwrap();

    assert!(
        registry
            .is_yanked("acme-tools", &Version::new(1, 0, 0))
            .unwrap()
    );
    assert!(
        registry
            .download("acme-tools", &Version::new(1, 0, 0))
            .is_ok()
    );
}
