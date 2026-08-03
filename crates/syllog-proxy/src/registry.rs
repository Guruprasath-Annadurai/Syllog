//! Immutable provider registry snapshots and ABI negotiation.

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;

use crate::{ModelRoute, ProviderAbiVersion, ProviderAdapter};

/// Provider registration or lookup failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderLookupError {
    /// Adapter ABI cannot run on this runtime.
    #[error(
        "provider '{provider}' ABI {found_major}.{found_minor} is incompatible with runtime ABI {supported_major}.{supported_minor}"
    )]
    AbiMismatch {
        /// Provider name.
        provider: String,
        /// Adapter major version.
        found_major: u16,
        /// Adapter minor version.
        found_minor: u16,
        /// Runtime major version.
        supported_major: u16,
        /// Runtime minor version.
        supported_minor: u16,
    },
    /// Provider name has already been assigned.
    #[error("provider '{provider}' is already registered")]
    Duplicate {
        /// Duplicate provider name.
        provider: String,
    },
    /// Route names no registered provider.
    #[error("unknown provider '{provider}'")]
    Unknown {
        /// Missing provider name.
        provider: String,
    },
}

/// Mutable registry builder whose published snapshots are immutable.
pub struct ProviderRegistry {
    supported: ProviderAbiVersion,
    adapters: BTreeMap<String, Arc<dyn ProviderAdapter>>,
}

impl ProviderRegistry {
    /// Creates an empty registry for one runtime ABI.
    #[must_use]
    pub fn new(supported: ProviderAbiVersion) -> Self {
        Self {
            supported,
            adapters: BTreeMap::new(),
        }
    }

    /// Registers one uniquely named compatible adapter.
    ///
    /// # Errors
    ///
    /// Rejects incompatible ABI versions and duplicate provider names.
    pub fn register(
        &mut self,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Result<(), ProviderLookupError> {
        let descriptor = adapter.descriptor();
        if descriptor.abi.major != self.supported.major
            || descriptor.abi.minor > self.supported.minor
        {
            return Err(ProviderLookupError::AbiMismatch {
                provider: descriptor.name.clone(),
                found_major: descriptor.abi.major,
                found_minor: descriptor.abi.minor,
                supported_major: self.supported.major,
                supported_minor: self.supported.minor,
            });
        }
        if self.adapters.contains_key(&descriptor.name) {
            return Err(ProviderLookupError::Duplicate {
                provider: descriptor.name.clone(),
            });
        }
        self.adapters.insert(descriptor.name.clone(), adapter);
        Ok(())
    }

    /// Number of successfully registered providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    /// Reports whether no providers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }

    /// Publishes an immutable copy-on-write registry snapshot.
    #[must_use]
    pub fn snapshot(&self) -> ProviderRegistrySnapshot {
        ProviderRegistrySnapshot {
            adapters: Arc::new(self.adapters.clone()),
        }
    }
}

/// Immutable provider registry view safe to share across requests.
#[derive(Clone)]
pub struct ProviderRegistrySnapshot {
    adapters: Arc<BTreeMap<String, Arc<dyn ProviderAdapter>>>,
}

impl ProviderRegistrySnapshot {
    /// Resolves the provider named by a model route.
    ///
    /// # Errors
    ///
    /// Returns `Unknown` when the snapshot has no exact provider name.
    pub fn resolve(
        &self,
        route: &ModelRoute,
    ) -> Result<Arc<dyn ProviderAdapter>, ProviderLookupError> {
        self.adapters
            .get(&route.provider)
            .cloned()
            .ok_or_else(|| ProviderLookupError::Unknown {
                provider: route.provider.clone(),
            })
    }
}
