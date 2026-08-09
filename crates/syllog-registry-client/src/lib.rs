//! Signed, content-addressed Syllog registry protocol.

mod transport;

pub use transport::*;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Current wire-format version for signed publication requests.
pub const PROTOCOL_VERSION: u32 = 1;

/// One canonical file in a package archive.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveFile {
    /// Slash-separated path relative to the extraction root.
    pub path: String,
    /// Exact file bytes.
    pub content: Vec<u8>,
}

impl ArchiveFile {
    /// Creates an archive entry; path validation occurs when assembling an archive.
    #[must_use]
    pub fn new(path: impl Into<String>, content: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            content,
        }
    }
}

/// Canonical package payload covered by the publisher signature.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchive {
    /// Package name.
    pub name: String,
    /// Immutable package version.
    pub version: Version,
    /// Files sorted by path.
    pub files: Vec<ArchiveFile>,
    /// Direct dependency requirements sorted by package name.
    pub dependencies: BTreeMap<String, String>,
}

impl PackageArchive {
    /// Validates and canonicalizes one package payload.
    ///
    /// # Errors
    ///
    /// Rejects invalid package names, empty archives, unsafe or duplicate paths.
    pub fn new(
        name: impl Into<String>,
        version: Version,
        mut files: Vec<ArchiveFile>,
        dependencies: BTreeMap<String, String>,
    ) -> Result<Self, RegistryError> {
        let name = name.into();
        validate_package_name(&name)?;
        if files.is_empty() {
            return Err(RegistryError::EmptyArchive);
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut paths = BTreeSet::new();
        for file in &files {
            validate_archive_path(&name, &version, &file.path)?;
            if !paths.insert(file.path.clone()) {
                return Err(RegistryError::DuplicateArchivePath {
                    path: file.path.clone(),
                });
            }
        }
        Ok(Self {
            name,
            version,
            files,
            dependencies,
        })
    }

    /// Computes the canonical lowercase SHA-256 content digest.
    ///
    /// # Errors
    ///
    /// Returns an error only if canonical JSON serialization fails.
    pub fn checksum(&self) -> Result<String, RegistryError> {
        let bytes = self.canonical_bytes()?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    /// Encodes the archive as deterministic JSON bytes for caching and transfer.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical JSON encoding fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RegistryError> {
        serde_json::to_vec(self).map_err(|error| RegistryError::Encoding {
            reason: error.to_string(),
        })
    }

    /// Decodes and revalidates canonical archive bytes.
    ///
    /// # Errors
    ///
    /// Rejects malformed encoding and all archive validation failures.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RegistryError> {
        let decoded: Self =
            serde_json::from_slice(bytes).map_err(|error| RegistryError::Encoding {
                reason: error.to_string(),
            })?;
        Self::new(
            decoded.name,
            decoded.version,
            decoded.files,
            decoded.dependencies,
        )
    }
}

/// Private publisher signing identity scoped to one registry namespace.
pub struct PublisherIdentity {
    namespace: String,
    signing_key: SigningKey,
}

impl PublisherIdentity {
    /// Deterministically constructs an identity from a protected 32-byte seed.
    #[must_use]
    pub fn from_seed(namespace: impl Into<String>, seed: [u8; 32]) -> Self {
        Self {
            namespace: namespace.into(),
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    /// Returns the public verification key; secret seed bytes are never exposed.
    #[must_use]
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Signs a versioned publication envelope including its anti-replay nonce.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical encoding fails.
    pub fn sign(
        &self,
        archive: PackageArchive,
        nonce: impl Into<String>,
    ) -> Result<PublishRequest, RegistryError> {
        let checksum = archive.checksum()?;
        let mut request = PublishRequest {
            protocol_version: PROTOCOL_VERSION,
            namespace: self.namespace.clone(),
            publisher_key: self.verifying_key(),
            nonce: nonce.into(),
            checksum,
            archive,
            signature: Vec::new(),
        };
        request.signature = self
            .signing_key
            .sign(&request.signing_bytes()?)
            .to_bytes()
            .to_vec();
        Ok(request)
    }
}

/// Complete signed registry publication request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublishRequest {
    /// Registry protocol version.
    pub protocol_version: u32,
    /// Publisher namespace.
    pub namespace: String,
    /// Ed25519 public key.
    pub publisher_key: [u8; 32],
    /// Publisher-scoped unique anti-replay nonce.
    pub nonce: String,
    /// Canonical archive checksum.
    pub checksum: String,
    /// Signed package content.
    pub archive: PackageArchive,
    /// Ed25519 signature over every preceding field.
    pub signature: Vec<u8>,
}

impl PublishRequest {
    fn signing_bytes(&self) -> Result<Vec<u8>, RegistryError> {
        #[derive(Serialize)]
        struct Unsigned<'a> {
            protocol_version: u32,
            namespace: &'a str,
            publisher_key: &'a [u8; 32],
            nonce: &'a str,
            checksum: &'a str,
            archive: &'a PackageArchive,
        }
        serde_json::to_vec(&Unsigned {
            protocol_version: self.protocol_version,
            namespace: &self.namespace,
            publisher_key: &self.publisher_key,
            nonce: &self.nonce,
            checksum: &self.checksum,
            archive: &self.archive,
        })
        .map_err(|error| RegistryError::Encoding {
            reason: error.to_string(),
        })
    }
}

/// Durable publication acknowledgement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishReceipt {
    /// Package name.
    pub name: String,
    /// Published version.
    pub version: Version,
    /// Verified content digest.
    pub checksum: String,
}

#[derive(Clone, Debug)]
struct StoredRelease {
    archive: PackageArchive,
    yanked: bool,
}

/// Deterministic no-network registry contract implementation.
#[derive(Clone, Debug, Default)]
pub struct LocalRegistry {
    authorized: BTreeMap<String, BTreeSet<[u8; 32]>>,
    used_nonces: BTreeSet<(String, String)>,
    releases: BTreeMap<(String, Version), StoredRelease>,
}

impl LocalRegistry {
    /// Creates an empty deny-by-default registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Grants one public key publication authority for `namespace`.
    pub fn authorize(&mut self, namespace: impl Into<String>, key: [u8; 32]) {
        self.authorized
            .entry(namespace.into())
            .or_default()
            .insert(key);
    }

    /// Verifies and immutably stores one signed release.
    ///
    /// # Errors
    ///
    /// Rejects protocol mismatch, replay, namespace or key violations,
    /// checksum/signature failure, and existing package versions.
    pub fn publish(&mut self, request: PublishRequest) -> Result<PublishReceipt, RegistryError> {
        if request.protocol_version != PROTOCOL_VERSION {
            return Err(RegistryError::ProtocolVersion {
                found: request.protocol_version,
            });
        }
        let actual = request.archive.checksum()?;
        if actual != request.checksum {
            return Err(RegistryError::ChecksumMismatch {
                expected: request.checksum,
                actual,
            });
        }
        if request.archive.name != request.namespace
            && !request
                .archive
                .name
                .starts_with(&format!("{}-", request.namespace))
        {
            return Err(RegistryError::NamespaceMismatch {
                namespace: request.namespace,
                package: request.archive.name,
            });
        }
        if !self
            .authorized
            .get(&request.namespace)
            .is_some_and(|keys| keys.contains(&request.publisher_key))
        {
            return Err(RegistryError::Unauthorized {
                namespace: request.namespace,
            });
        }
        let nonce_key = (request.namespace.clone(), request.nonce.clone());
        if self.used_nonces.contains(&nonce_key) {
            return Err(RegistryError::Replay {
                namespace: request.namespace,
                nonce: request.nonce,
            });
        }
        let key = (
            request.archive.name.clone(),
            request.archive.version.clone(),
        );
        if self.releases.contains_key(&key) {
            return Err(RegistryError::ImmutableVersion {
                package: key.0,
                version: key.1,
            });
        }
        let verifying_key = VerifyingKey::from_bytes(&request.publisher_key).map_err(|_| {
            RegistryError::InvalidSignature {
                package: request.archive.name.clone(),
            }
        })?;
        let signature = Signature::from_slice(&request.signature).map_err(|_| {
            RegistryError::InvalidSignature {
                package: request.archive.name.clone(),
            }
        })?;
        verifying_key
            .verify(&request.signing_bytes()?, &signature)
            .map_err(|_| RegistryError::InvalidSignature {
                package: request.archive.name.clone(),
            })?;

        self.used_nonces.insert(nonce_key);
        self.releases.insert(
            key,
            StoredRelease {
                archive: request.archive.clone(),
                yanked: false,
            },
        );
        Ok(PublishReceipt {
            name: request.archive.name,
            version: request.archive.version,
            checksum: request.checksum,
        })
    }

    /// Retrieves immutable package content, including yanked locked versions.
    ///
    /// # Errors
    ///
    /// Returns an error when the exact release does not exist.
    pub fn download(&self, name: &str, version: &Version) -> Result<PackageArchive, RegistryError> {
        self.releases
            .get(&(name.to_owned(), version.clone()))
            .map(|stored| stored.archive.clone())
            .ok_or_else(|| RegistryError::NotFound {
                package: name.to_owned(),
                version: version.clone(),
            })
    }

    /// Marks a version as unavailable to new resolutions without deleting it.
    ///
    /// # Errors
    ///
    /// Returns an error when the exact release does not exist.
    pub fn yank(&mut self, name: &str, version: &Version) -> Result<(), RegistryError> {
        let stored = self
            .releases
            .get_mut(&(name.to_owned(), version.clone()))
            .ok_or_else(|| RegistryError::NotFound {
                package: name.to_owned(),
                version: version.clone(),
            })?;
        stored.yanked = true;
        Ok(())
    }

    /// Reports whether an exact release is yanked.
    ///
    /// # Errors
    ///
    /// Returns an error when the exact release does not exist.
    pub fn is_yanked(&self, name: &str, version: &Version) -> Result<bool, RegistryError> {
        self.releases
            .get(&(name.to_owned(), version.clone()))
            .map(|stored| stored.yanked)
            .ok_or_else(|| RegistryError::NotFound {
                package: name.to_owned(),
                version: version.clone(),
            })
    }
}

/// Stable registry validation or protocol failure.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RegistryError {
    /// Unsupported protocol version.
    #[error("unsupported registry protocol version {found}")]
    ProtocolVersion {
        /// Received version.
        found: u32,
    },
    /// Package name is invalid.
    #[error("invalid package name '{name}'")]
    InvalidPackageName {
        /// Invalid name.
        name: String,
    },
    /// Package archive has no files.
    #[error("package archive is empty")]
    EmptyArchive,
    /// Archive entry can escape or is not portable.
    #[error("unsafe archive path '{path}' in {package} {version}")]
    UnsafeArchivePath {
        /// Package name.
        package: String,
        /// Package version.
        version: Version,
        /// Rejected path.
        path: String,
    },
    /// Two archive entries have the same canonical path.
    #[error("duplicate archive path '{path}'")]
    DuplicateArchivePath {
        /// Duplicate path.
        path: String,
    },
    /// Canonical encoding failed.
    #[error("registry encoding failed: {reason}")]
    Encoding {
        /// Encoder explanation.
        reason: String,
    },
    /// Content does not match signed metadata.
    #[error("checksum mismatch: expected {expected}, computed {actual}")]
    ChecksumMismatch {
        /// Signed digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },
    /// Publisher namespace does not own the requested package name.
    #[error("namespace '{namespace}' cannot publish package '{package}'")]
    NamespaceMismatch {
        /// Publisher namespace.
        namespace: String,
        /// Requested package.
        package: String,
    },
    /// Publisher key is not authorized.
    #[error("publisher key is unauthorized for namespace '{namespace}'")]
    Unauthorized {
        /// Requested namespace.
        namespace: String,
    },
    /// Nonce was previously accepted.
    #[error("publication nonce '{nonce}' was already used for namespace '{namespace}'")]
    Replay {
        /// Publisher namespace.
        namespace: String,
        /// Reused nonce.
        nonce: String,
    },
    /// Package name/version is already permanently assigned.
    #[error("package '{package}' version {version} already exists and is immutable")]
    ImmutableVersion {
        /// Package name.
        package: String,
        /// Existing version.
        version: Version,
    },
    /// Signature bytes or verification are invalid.
    #[error("invalid publisher signature for package '{package}'")]
    InvalidSignature {
        /// Package name.
        package: String,
    },
    /// Exact release does not exist.
    #[error("package '{package}' version {version} was not found")]
    NotFound {
        /// Package name.
        package: String,
        /// Requested version.
        version: Version,
    },
}

fn validate_package_name(name: &str) -> Result<(), RegistryError> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let valid_rest = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if valid_start && valid_rest && !name.ends_with('-') && !name.contains("--") {
        Ok(())
    } else {
        Err(RegistryError::InvalidPackageName {
            name: name.to_owned(),
        })
    }
}

fn validate_archive_path(
    package: &str,
    version: &Version,
    path: &str,
) -> Result<(), RegistryError> {
    let portable = !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if portable {
        Ok(())
    } else {
        Err(RegistryError::UnsafeArchivePath {
            package: package.to_owned(),
            version: version.clone(),
            path: path.to_owned(),
        })
    }
}
