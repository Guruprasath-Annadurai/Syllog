//! Versioned provider ABI and credential-safe adapter boundary.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize, Serializer};

use crate::TokenSink;

/// ABI version negotiated between runtime and provider adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProviderAbiVersion {
    /// Breaking interface generation.
    pub major: u16,
    /// Backward-compatible interface feature level.
    pub minor: u16,
}

/// Credential shape required by an adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// No external secret.
    None,
    /// HTTP bearer token held behind a secret capability.
    BearerToken,
    /// Local socket or process capability.
    LocalCapability,
}

/// Immutable provider ABI descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Stable registry name.
    pub name: String,
    /// Adapter ABI implemented by this provider.
    pub abi: ProviderAbiVersion,
    /// Required credential handle kind.
    pub credentials: CredentialKind,
}

/// Secret whose formatting and serialization are always redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wraps secret material. Callers cannot retrieve it through formatting or
    /// serialization APIs.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Executes an adapter-owned closure with transient access to the secret.
    ///
    /// This avoids copying the secret into general-purpose request structures.
    pub fn with_exposed<T>(&self, operation: impl FnOnce(&str) -> T) -> T {
        operation(&self.0)
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Serialize for SecretValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

/// A concrete provider/model endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRoute {
    /// Stable route identifier.
    pub name: String,
    /// Provider adapter name.
    pub provider: String,
    /// Provider-specific model identifier.
    pub model: String,
}

/// One model invocation routed through a provider adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// Selected provider/model route.
    pub route: ModelRoute,
    /// Input text supplied to the model.
    pub input: String,
}

/// Normalized provider failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorCategory {
    /// Explicit caller or scope cancellation.
    Cancelled,
    /// Deadline elapsed.
    Timeout,
    /// Provider rejected credentials or authority.
    Authentication,
    /// Provider requested retry/backoff.
    RateLimited,
    /// Malformed or incompatible provider response.
    Protocol,
    /// Transient upstream failure.
    Unavailable,
}

/// Provider failure surfaced in a pipeline stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    category: ProviderErrorCategory,
    message: String,
}

impl ProviderError {
    /// Creates a transient upstream provider error for backward compatibility.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self::categorized(ProviderErrorCategory::Unavailable, message)
    }

    /// Creates a normalized provider failure.
    #[must_use]
    pub fn categorized(category: ProviderErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// Returns the stable category used by retry and circuit policies.
    #[must_use]
    pub fn category(&self) -> ProviderErrorCategory {
        self.category
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

/// A boxed asynchronous provider invocation.
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ProviderError>> + Send + 'a>>;

/// Adapter boundary implemented by model providers.
pub trait ProviderAdapter: Send + Sync {
    /// Describes the adapter ABI, identity, and credential contract.
    fn descriptor(&self) -> &ProviderDescriptor;

    /// Streams one request into the executor-owned bounded sink.
    fn stream(&self, request: ModelRequest, sink: TokenSink) -> ProviderFuture<'_>;
}

/// Deterministic provider used by tests and offline development.
#[derive(Debug, Clone)]
pub struct MockProvider {
    descriptor: ProviderDescriptor,
    events: Vec<Result<crate::Token, ProviderError>>,
}

impl MockProvider {
    /// Creates a provider that emits supplied token fragments under the default
    /// `mock` ABI descriptor.
    pub fn tokens<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::with_descriptor(default_mock_descriptor(), tokens)
    }

    /// Creates a provider with an exact descriptor and successful token list.
    pub fn with_descriptor<I, S>(descriptor: ProviderDescriptor, tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            descriptor,
            events: tokens
                .into_iter()
                .map(|token| Ok(crate::Token::from(token.into())))
                .collect(),
        }
    }

    /// Creates a provider from an exact sequence of token and error events.
    pub fn scripted<I>(events: I) -> Self
    where
        I: IntoIterator<Item = Result<crate::Token, ProviderError>>,
    {
        Self {
            descriptor: default_mock_descriptor(),
            events: events.into_iter().collect(),
        }
    }
}

impl ProviderAdapter for MockProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    fn stream(&self, _request: ModelRequest, sink: TokenSink) -> ProviderFuture<'_> {
        Box::pin(async move {
            for event in self.events.iter().cloned() {
                match event {
                    Ok(token) => sink.send(Ok(token)).await?,
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })
    }
}

fn default_mock_descriptor() -> ProviderDescriptor {
    ProviderDescriptor {
        name: "mock".into(),
        abi: ProviderAbiVersion { major: 1, minor: 0 },
        credentials: CredentialKind::None,
    }
}
