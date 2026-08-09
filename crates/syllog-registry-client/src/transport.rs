//! Authenticated HTTP registry transport with resumable archive downloads.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use reqwest::header::{AUTHORIZATION, CONTENT_RANGE, RANGE};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::{PackageArchive, PublishRequest};

/// Bearer credential that never exposes its contents through formatting.
#[derive(Clone)]
pub struct RegistryCredential(Arc<str>);

impl RegistryCredential {
    /// Wraps an opaque registry bearer token.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Arc::from(value.into()))
    }
}

impl fmt::Debug for RegistryCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RegistryCredential([REDACTED])")
    }
}

/// Signed build provenance uploaded atomically with a package publication.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceStatement {
    /// Versioned provenance schema.
    pub schema_version: u32,
    /// Immutable package archive digest.
    pub archive_checksum: String,
    /// Compiler identity used for the build.
    pub compiler: String,
    /// Source revision or content identity.
    pub source_revision: String,
}

/// Atomic network publication envelope.
#[derive(Clone, Debug, Serialize)]
pub struct RemotePublication<'a> {
    /// Publisher-signed immutable release.
    pub publication: &'a PublishRequest,
    /// Provenance bound to the exact archive checksum.
    pub provenance: &'a ProvenanceStatement,
}

/// Receipt returned by a network registry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemotePublishReceipt {
    /// Published package.
    pub name: String,
    /// Immutable version.
    pub version: Version,
    /// Stored archive checksum.
    pub checksum: String,
    /// Server-assigned append-only provenance identifier.
    pub provenance_id: String,
}

/// Registry network or integrity failure.
#[derive(Debug, thiserror::Error)]
pub enum RegistryTransportError {
    /// Client configuration was invalid.
    #[error("invalid registry client configuration: {0}")]
    Configuration(String),
    /// HTTP request or response streaming failed.
    #[error("registry transport failed: {0}")]
    Http(#[from] reqwest::Error),
    /// Registry returned an unsuccessful status.
    #[error("registry returned HTTP {status}: {message}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// Bounded response explanation.
        message: String,
    },
    /// Resumption response did not describe the requested offset.
    #[error("invalid resume response: expected byte offset {expected}, received {received:?}")]
    InvalidRange {
        /// Requested starting offset.
        expected: u64,
        /// Returned `Content-Range` value.
        received: Option<String>,
    },
    /// Downloaded content failed its locked digest.
    #[error("download checksum mismatch: expected {expected}, computed {actual}")]
    Checksum {
        /// Locked digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },
    /// Archive or provenance identity was inconsistent.
    #[error("registry integrity violation: {0}")]
    Integrity(String),
    /// Local durable-file operation failed.
    #[error("registry file operation failed for '{}': {source}", path.display())]
    Io {
        /// Affected file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Downloaded archive encoding was invalid.
    #[error("downloaded package archive is invalid: {0}")]
    Archive(#[from] crate::RegistryError),
}

/// HTTPS registry client with bounded deadlines and explicit credentials.
#[derive(Clone)]
pub struct HttpRegistryClient {
    base_url: Arc<str>,
    credential: RegistryCredential,
    client: reqwest::Client,
}

impl fmt::Debug for HttpRegistryClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRegistryClient")
            .field("base_url", &self.base_url)
            .field("credential", &self.credential)
            .finish_non_exhaustive()
    }
}

impl HttpRegistryClient {
    /// Creates a client with a total request deadline and no ambient proxy.
    ///
    /// # Errors
    ///
    /// Rejects invalid URLs, zero deadlines, and HTTP client configuration failures.
    pub fn new(
        base_url: impl Into<String>,
        credential: RegistryCredential,
        deadline: Duration,
    ) -> Result<Self, RegistryTransportError> {
        if deadline.is_zero() {
            return Err(RegistryTransportError::Configuration(
                "deadline must be greater than zero".into(),
            ));
        }
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|error| RegistryTransportError::Configuration(error.to_string()))?;
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(RegistryTransportError::Configuration(
                "registry URL must not contain credentials, a query, or a fragment".into(),
            ));
        }
        let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if parsed.scheme() != "https" && !loopback {
            return Err(RegistryTransportError::Configuration(
                "registry URL must use HTTPS except for loopback contract servers".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(deadline)
            .no_proxy()
            .build()?;
        Ok(Self {
            base_url: Arc::from(base_url),
            credential,
            client,
        })
    }

    /// Atomically uploads a signed package and checksum-bound provenance.
    ///
    /// # Errors
    ///
    /// Returns a typed transport/status/integrity failure.
    pub async fn publish(
        &self,
        publication: &PublishRequest,
        provenance: &ProvenanceStatement,
    ) -> Result<RemotePublishReceipt, RegistryTransportError> {
        if provenance.archive_checksum != publication.checksum {
            return Err(RegistryTransportError::Integrity(
                "provenance checksum does not match publication".into(),
            ));
        }
        let response = self
            .authorized(
                self.client
                    .post(format!("{}/v1/publications", self.base_url)),
            )
            .json(&RemotePublication {
                publication,
                provenance,
            })
            .send()
            .await?;
        let response = successful(response).await?;
        let receipt: RemotePublishReceipt = response.json().await?;
        if receipt.name != publication.archive.name
            || receipt.version != publication.archive.version
            || receipt.checksum != publication.checksum
        {
            return Err(RegistryTransportError::Integrity(
                "publication receipt does not match signed request".into(),
            ));
        }
        Ok(receipt)
    }

    /// Resumes an interrupted archive download and atomically publishes the
    /// completed, checksum-verified destination.
    ///
    /// # Errors
    ///
    /// Returns a typed network, range, filesystem, archive, or checksum failure.
    pub async fn download_resumable(
        &self,
        name: &str,
        version: &Version,
        checksum: &str,
        destination: &Path,
    ) -> Result<PackageArchive, RegistryTransportError> {
        if !valid_package_name(name) {
            return Err(RegistryTransportError::Configuration(format!(
                "invalid package name '{name}'"
            )));
        }
        if checksum.len() != 64
            || !checksum
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RegistryTransportError::Configuration(
                "package checksum must be 64 lowercase hexadecimal characters".into(),
            ));
        }
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|source| {
                RegistryTransportError::Io {
                    path: parent.to_owned(),
                    source,
                }
            })?;
        }
        let partial = partial_path(destination);
        let offset = match tokio::fs::metadata(&partial).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(source) => {
                return Err(RegistryTransportError::Io {
                    path: partial,
                    source,
                });
            }
        };
        let url = format!("{}/v1/packages/{name}/{version}/archive", self.base_url);
        let mut request = self.authorized(self.client.get(url));
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let append = offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if offset > 0 && append {
            let received = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned);
            let expected_prefix = format!("bytes {offset}-");
            if !received
                .as_deref()
                .is_some_and(|value| value.starts_with(&expected_prefix))
            {
                return Err(RegistryTransportError::InvalidRange {
                    expected: offset,
                    received,
                });
            }
        }
        write_response(&partial, response, append).await?;
        let archive = verify_partial(&partial, name, version, checksum).await?;
        tokio::fs::rename(&partial, destination)
            .await
            .map_err(|source| RegistryTransportError::Io {
                path: destination.to_owned(),
                source,
            })?;
        Ok(archive)
    }

    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.header(AUTHORIZATION, format!("Bearer {}", self.credential.0))
    }
}

async fn write_response(
    partial: &Path,
    response: reqwest::Response,
    append: bool,
) -> Result<(), RegistryTransportError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).write(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    let mut file = options
        .open(partial)
        .await
        .map_err(|source| RegistryTransportError::Io {
            path: partial.to_owned(),
            source,
        })?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?)
            .await
            .map_err(|source| RegistryTransportError::Io {
                path: partial.to_owned(),
                source,
            })?;
    }
    file.sync_all()
        .await
        .map_err(|source| RegistryTransportError::Io {
            path: partial.to_owned(),
            source,
        })
}

async fn verify_partial(
    partial: &Path,
    name: &str,
    version: &Version,
    checksum: &str,
) -> Result<PackageArchive, RegistryTransportError> {
    let bytes = tokio::fs::read(partial)
        .await
        .map_err(|source| RegistryTransportError::Io {
            path: partial.to_owned(),
            source,
        })?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != checksum {
        return Err(RegistryTransportError::Checksum {
            expected: checksum.into(),
            actual,
        });
    }
    let archive = PackageArchive::from_bytes(&bytes)?;
    if archive.name != name || archive.version != *version {
        return Err(RegistryTransportError::Integrity(
            "downloaded archive identity does not match request".into(),
        ));
    }
    Ok(archive)
}

async fn successful(
    response: reqwest::Response,
) -> Result<reqwest::Response, RegistryTransportError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(status_error(response).await)
    }
}

async fn status_error(response: reqwest::Response) -> RegistryTransportError {
    let status = response.status().as_u16();
    let mut bytes = Vec::with_capacity(4096);
    let mut stream = response.bytes_stream();
    while bytes.len() < 4096 {
        let Some(chunk) = stream.next().await else {
            break;
        };
        let Ok(chunk) = chunk else {
            break;
        };
        let remaining = 4096 - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let message = if bytes.is_empty() {
        "unreadable or empty response".into()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    RegistryTransportError::Status { status, message }
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

fn valid_package_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !name.ends_with('-')
        && !name.contains("--")
}
