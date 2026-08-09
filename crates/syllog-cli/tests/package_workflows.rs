//! End-to-end offline package command contracts.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

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
    let manifest = root.join("Syllog.toml");
    let source = std::fs::read_to_string(&manifest).unwrap().replace(
        "[capabilities]",
        "[dependencies]\nacme-tools = \"=1.2.3\"\n\n[capabilities]",
    );
    std::fs::write(&manifest, source).unwrap();
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

#[test]
fn project_runs_from_verified_lockfile_cache_and_then_vendor_only() {
    let directory = project();
    let root = directory.path().join("acme-app");
    let manifest = root.join("Syllog.toml");
    let source = std::fs::read_to_string(&manifest).unwrap().replace(
        "[capabilities]",
        "[dependencies]\nacme-tools = \"=1.2.3\"\n\n[capabilities]",
    );
    std::fs::write(&manifest, source).unwrap();
    std::fs::write(
        root.join("src/main.syl"),
        "module app;\nuse acme_tools::tool;\nfn main() -> I64 { tool() }\n",
    )
    .unwrap();
    let archive = PackageArchive::new(
        "acme-tools",
        Version::new(1, 2, 3),
        vec![ArchiveFile::new(
            "src/lib.syl",
            b"module acme_tools;\npub fn tool() -> I64 { 7 }\n".to_vec(),
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

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args(["run", ".", "--memory-bytes", "65536"])
            .current_dir(&root)
            .output()
            .unwrap()
    };
    let cached = run();
    assert!(
        cached.status.success(),
        "{}",
        String::from_utf8_lossy(&cached.stderr)
    );
    assert_eq!(cached.stdout, b"7\n");

    let vendor = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .arg("vendor")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        vendor.status.success(),
        "{}",
        String::from_utf8_lossy(&vendor.stderr)
    );
    std::fs::rename(
        root.join(".syllog/cache"),
        root.join(".syllog/cache-disabled"),
    )
    .unwrap();

    let vendored = run();
    assert!(
        vendored.status.success(),
        "{}",
        String::from_utf8_lossy(&vendored.stderr)
    );
    assert_eq!(vendored.stdout, b"7\n");
}

#[test]
fn project_build_rejects_lockfile_and_archive_dependency_divergence() {
    let directory = project();
    let root = directory.path().join("acme-app");
    let manifest = root.join("Syllog.toml");
    let source = std::fs::read_to_string(&manifest).unwrap().replace(
        "[capabilities]",
        "[dependencies]\nacme-tools = \"=1.2.3\"\n\n[capabilities]",
    );
    std::fs::write(&manifest, source).unwrap();
    let archive = PackageArchive::new(
        "acme-tools",
        Version::new(1, 2, 3),
        vec![ArchiveFile::new(
            "src/lib.syl",
            b"module acme_tools;\npub fn tool() -> I64 { 7 }\n".to_vec(),
        )],
        BTreeMap::from([("transitive-tool".into(), "^1".into())]),
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
        .args(["build", ".", "--output", "target/app.wasm"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not match Syllog.lock"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn non_dry_run_publish_uploads_signed_archive_and_provenance() {
    let directory = project();
    let root = directory.path().join("acme-app");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let (headers, body) = read_http_request(&mut stream);
        assert!(headers.contains("authorization: bearer test-token"));
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            envelope["provenance"]["source_revision"],
            "git:test-revision"
        );
        assert_eq!(
            envelope["provenance"]["archive_checksum"],
            envelope["publication"]["checksum"]
        );
        let receipt = serde_json::to_vec(&serde_json::json!({
            "name": envelope["publication"]["archive"]["name"],
            "version": envelope["publication"]["archive"]["version"],
            "checksum": envelope["publication"]["checksum"],
            "provenance_id": "prov-cli-1"
        }))
        .unwrap();
        write!(
            stream,
            "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            receipt.len()
        )
        .unwrap();
        stream.write_all(&receipt).unwrap();
    });

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args([
            "publish",
            "--registry",
            &format!("http://{address}"),
            "--nonce",
            "nonce-cli-1",
            "--source-revision",
            "git:test-revision",
        ])
        .env("SYLLOG_REGISTRY_TOKEN", "test-token")
        .env("SYLLOG_PUBLISHER_SEED_HEX", "07".repeat(32))
        .current_dir(&root)
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-token"));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["provenance_id"], "prov-cli-1");
}

fn read_http_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "client closed before completing request");
        bytes.extend_from_slice(&buffer[..count]);
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap();
        if bytes.len() >= header_end + content_length {
            return (
                headers.to_ascii_lowercase(),
                bytes[header_end..header_end + content_length].to_vec(),
            );
        }
    }
}
