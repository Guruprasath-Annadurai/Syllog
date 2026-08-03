//! Asynchronous, allocation-conscious model routing for Syllog agents.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

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

/// One streaming text fragment from a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// UTF-8 token text.
    pub text: String,
}

impl From<&str> for Token {
    fn from(text: &str) -> Self {
        Self { text: text.into() }
    }
}

impl From<String> for Token {
    fn from(text: String) -> Self {
        Self { text }
    }
}

/// Provider failure surfaced in a pipeline stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    message: String,
}

impl ProviderError {
    /// Creates a provider error with a stable human-readable message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

/// A boxed asynchronous provider invocation.
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ProviderError>> + Send + 'a>>;

/// Bounded channel used by adapters to emit token events.
pub type TokenSender = mpsc::Sender<Result<Token, ProviderError>>;

/// Bounded token stream returned by the pipeline executor.
pub type TokenStream = mpsc::Receiver<Result<Token, ProviderError>>;

/// Adapter boundary implemented by model providers.
pub trait ProviderAdapter: Send + Sync {
    /// Streams one model request into the executor-owned bounded channel.
    fn stream(&self, request: ModelRequest, output: TokenSender) -> ProviderFuture<'_>;
}

/// Deterministic provider used by tests and offline development.
#[derive(Debug, Clone)]
pub struct MockProvider {
    events: Vec<Result<Token, ProviderError>>,
}

impl MockProvider {
    /// Creates a provider that emits the supplied token fragments in order.
    pub fn tokens<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            events: tokens
                .into_iter()
                .map(|token| Ok(Token::from(token.into())))
                .collect(),
        }
    }

    /// Creates a provider from an exact sequence of token and error events.
    pub fn scripted<I>(events: I) -> Self
    where
        I: IntoIterator<Item = Result<Token, ProviderError>>,
    {
        Self {
            events: events.into_iter().collect(),
        }
    }
}

impl ProviderAdapter for MockProvider {
    fn stream(&self, _request: ModelRequest, output: TokenSender) -> ProviderFuture<'_> {
        Box::pin(async move {
            for event in self.events.iter().cloned() {
                match event {
                    Ok(token) => {
                        if output.send(Ok(token)).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })
    }
}

/// Invalid streaming executor configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineConfigError;

impl std::fmt::Display for PipelineConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("pipeline channel capacity must be greater than zero")
    }
}

impl std::error::Error for PipelineConfigError {}

/// Executes provider calls on Tokio with a bounded token channel.
#[derive(Debug, Clone, Copy)]
pub struct PipelineExecutor {
    channel_capacity: usize,
}

impl PipelineExecutor {
    /// Creates an executor with the maximum number of buffered token events.
    ///
    /// # Errors
    ///
    /// Returns an error when `channel_capacity` is zero.
    pub fn new(channel_capacity: usize) -> Result<Self, PipelineConfigError> {
        if channel_capacity == 0 {
            return Err(PipelineConfigError);
        }
        Ok(Self { channel_capacity })
    }

    /// Starts one provider invocation and returns its bounded token stream.
    #[must_use]
    pub fn execute(
        &self,
        provider: Arc<dyn ProviderAdapter>,
        request: ModelRequest,
    ) -> TokenStream {
        let (output, stream) = mpsc::channel(self.channel_capacity);
        let terminal = output.clone();
        tokio::spawn(async move {
            if let Err(error) = provider.stream(request, output).await {
                let _ = terminal.send(Err(error)).await;
            }
        });
        stream
    }
}

/// Ordered routing table with an in-memory circuit-breaker view.
#[derive(Debug, Default)]
pub struct Router {
    routes: Vec<ModelRoute>,
    unavailable: HashSet<String>,
}

impl Router {
    /// Creates a router whose first route is preferred and later routes are fallbacks.
    #[must_use]
    pub fn new(routes: Vec<ModelRoute>) -> Self {
        Self {
            routes,
            unavailable: HashSet::new(),
        }
    }

    /// Marks a route unavailable until it is explicitly restored.
    pub fn trip(&mut self, name: impl Into<String>) {
        self.unavailable.insert(name.into());
    }

    /// Restores a previously tripped route.
    pub fn restore(&mut self, name: &str) {
        self.unavailable.remove(name);
    }

    /// Selects the first route whose circuit is closed.
    #[must_use]
    pub fn select(&self) -> Option<&ModelRoute> {
        self.routes
            .iter()
            .find(|route| !self.unavailable.contains(&route.name))
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelRoute, Router};

    fn route(name: &str) -> ModelRoute {
        ModelRoute {
            name: name.into(),
            provider: "test".into(),
            model: name.into(),
        }
    }

    #[test]
    fn tripped_primary_uses_fallback_until_restored() {
        let mut router = Router::new(vec![route("primary"), route("fallback")]);
        router.trip("primary");
        assert_eq!(
            router.select().map(|route| route.name.as_str()),
            Some("fallback")
        );
        router.restore("primary");
        assert_eq!(
            router.select().map(|route| route.name.as_str()),
            Some("primary")
        );
    }
}
