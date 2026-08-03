//! Verified content-addressed package archive storage.

use std::io::Write as _;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

/// Failure while validating or accessing cached package content.
#[derive(Debug, Error)]
pub enum CacheError {
    /// The requested digest is not canonical lowercase SHA-256.
    #[error("invalid SHA-256 cache key '{0}'")]
    InvalidKey(String),
    /// Supplied or loaded bytes do not match the requested digest.
    #[error("cache checksum mismatch: expected {expected}, computed {actual}")]
    ChecksumMismatch {
        /// Requested checksum.
        expected: String,
        /// Computed checksum.
        actual: String,
    },
    /// A filesystem operation failed.
    #[error("cache I/O failed for '{}': {source}", path.display())]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },
}

/// A local immutable cache keyed exclusively by package content digest.
#[derive(Clone, Debug)]
pub struct ContentAddressedCache {
    root: PathBuf,
}

impl ContentAddressedCache {
    /// Creates a cache rooted at `root`. Directories are created lazily.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the canonical location for a validated digest.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical or path-like keys.
    pub fn path_for(&self, checksum: &str) -> Result<PathBuf, CacheError> {
        validate_key(checksum)?;
        Ok(self.root.join("sha256").join(checksum))
    }

    /// Atomically stores bytes after verifying their requested digest.
    ///
    /// Existing immutable content is verified and retained.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid keys, mismatched content, or I/O failure.
    pub fn store(&self, checksum: &str, content: &[u8]) -> Result<PathBuf, CacheError> {
        verify(checksum, content)?;
        let destination = self.path_for(checksum)?;
        if destination.exists() {
            let existing = std::fs::read(&destination).map_err(|source| CacheError::Io {
                path: destination.clone(),
                source,
            })?;
            verify(checksum, &existing)?;
            return Ok(destination);
        }
        let parent = self.root.join("sha256");
        std::fs::create_dir_all(&parent).map_err(|source| CacheError::Io {
            path: parent.clone(),
            source,
        })?;
        let mut temporary = NamedTempFile::new_in(&parent).map_err(|source| CacheError::Io {
            path: destination.clone(),
            source,
        })?;
        temporary
            .write_all(content)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|source| CacheError::Io {
                path: destination.clone(),
                source,
            })?;
        match temporary.persist_noclobber(&destination) {
            Ok(_) => Ok(destination),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = std::fs::read(&destination).map_err(|source| CacheError::Io {
                    path: destination.clone(),
                    source,
                })?;
                verify(checksum, &existing)?;
                Ok(destination)
            }
            Err(error) => Err(CacheError::Io {
                path: destination,
                source: error.error,
            }),
        }
    }

    /// Loads and re-verifies immutable cached bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid keys, missing content, I/O failure, or
    /// on-disk corruption.
    pub fn load(&self, checksum: &str) -> Result<Vec<u8>, CacheError> {
        let path = self.path_for(checksum)?;
        let content = std::fs::read(&path).map_err(|source| CacheError::Io { path, source })?;
        verify(checksum, &content)?;
        Ok(content)
    }
}

fn validate_key(checksum: &str) -> Result<(), CacheError> {
    if checksum.len() == 64
        && checksum
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(CacheError::InvalidKey(checksum.to_owned()))
    }
}

fn verify(checksum: &str, content: &[u8]) -> Result<(), CacheError> {
    validate_key(checksum)?;
    let actual = format!("{:x}", Sha256::digest(content));
    if checksum == actual {
        Ok(())
    } else {
        Err(CacheError::ChecksumMismatch {
            expected: checksum.to_owned(),
            actual,
        })
    }
}
