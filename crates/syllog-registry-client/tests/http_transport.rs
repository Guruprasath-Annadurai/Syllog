//! Loopback HTTP contracts for resumable retrieval and atomic publication.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use semver::Version;
use serde_json::{Value, json};
use syllog_registry_client::{
    ArchiveFile, HttpRegistryClient, PackageArchive, ProvenanceStatement, PublisherIdentity,
    RegistryCredential,
};

#[derive(Clone)]
struct ContractState {
    archive: Arc<Vec<u8>>,
    range: Arc<Mutex<Option<String>>>,
    publication: Arc<Mutex<Option<Value>>>,
}

#[tokio::test]
async fn resumes_verified_archives_and_atomically_uploads_signed_provenance() {
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
    let state = ContractState {
        archive: Arc::new(bytes.clone()),
        range: Arc::new(Mutex::new(None)),
        publication: Arc::new(Mutex::new(None)),
    };
    let app = Router::new()
        .route("/v1/packages/{name}/{version}/archive", get(download))
        .route("/v1/publications", post(publish))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = HttpRegistryClient::new(
        format!("http://{address}"),
        RegistryCredential::new("registry-secret"),
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(!format!("{client:?}").contains("registry-secret"));
    let directory = tempfile::tempdir().unwrap();
    let invalid_name = client
        .download_resumable(
            "../escape",
            &Version::new(1, 2, 3),
            &checksum,
            &directory.path().join("unused"),
        )
        .await
        .unwrap_err();
    assert!(invalid_name.to_string().contains("invalid package name"));
    let destination = directory
        .path()
        .join("cache")
        .join("sha256")
        .join("archive.sylpkg");
    let partial = destination.with_file_name("archive.sylpkg.part");
    std::fs::create_dir_all(partial.parent().unwrap()).unwrap();
    std::fs::write(partial, &bytes[..17]).unwrap();

    let downloaded = client
        .download_resumable(
            "acme-tools",
            &Version::new(1, 2, 3),
            &checksum,
            &destination,
        )
        .await
        .unwrap();
    assert_eq!(downloaded, archive);
    assert_eq!(std::fs::read(destination).unwrap(), bytes);
    assert_eq!(state.range.lock().unwrap().as_deref(), Some("bytes=17-"));

    let fresh_destination = directory
        .path()
        .join("fresh-cache")
        .join("sha256")
        .join("archive.sylpkg");
    let fresh_download = client
        .download_resumable(
            "acme-tools",
            &Version::new(1, 2, 3),
            &checksum,
            &fresh_destination,
        )
        .await
        .unwrap();
    assert_eq!(fresh_download, archive);
    assert_eq!(std::fs::read(fresh_destination).unwrap(), bytes);

    let identity = PublisherIdentity::from_seed("acme-tools", [7; 32]);
    let request = identity.sign(archive, "nonce-1").unwrap();
    let provenance = ProvenanceStatement {
        schema_version: 1,
        archive_checksum: checksum.clone(),
        compiler: "syllog 0.1.0".into(),
        source_revision: "git:abc123".into(),
    };
    let receipt = client.publish(&request, &provenance).await.unwrap();
    assert_eq!(receipt.checksum, checksum);
    let captured = state.publication.lock().unwrap();
    assert_eq!(
        captured.as_ref().unwrap()["provenance"]["source_revision"],
        "git:abc123"
    );

    server.abort();
}

async fn download(State(state): State<ContractState>, headers: HeaderMap) -> Response<Body> {
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("bytes=0-")
        .to_owned();
    *state.range.lock().unwrap() = Some(range.clone());
    let offset = range
        .strip_prefix("bytes=")
        .and_then(|value| value.strip_suffix('-'))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    Response::builder()
        .status(if offset == 0 {
            StatusCode::OK
        } else {
            StatusCode::PARTIAL_CONTENT
        })
        .header(
            header::CONTENT_RANGE,
            format!(
                "bytes {offset}-{}/{}",
                state.archive.len() - 1,
                state.archive.len()
            ),
        )
        .body(Body::from(state.archive[offset..].to_vec()))
        .unwrap()
}

async fn publish(
    State(state): State<ContractState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert_eq!(
        headers.get(header::AUTHORIZATION).unwrap(),
        "Bearer registry-secret"
    );
    *state.publication.lock().unwrap() = Some(body.clone());
    (
        StatusCode::CREATED,
        Json(json!({
            "name": body["publication"]["archive"]["name"],
            "version": body["publication"]["archive"]["version"],
            "checksum": body["publication"]["checksum"],
            "provenance_id": "prov-1"
        })),
    )
}
