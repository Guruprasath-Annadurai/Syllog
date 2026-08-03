//! Bounded, ordered, cancellation-safe token streaming.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc};

use crate::{ModelRequest, ProviderAdapter, ProviderError, ProviderErrorCategory};

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

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

/// Cloneable explicit cancellation handle for one provider invocation.
#[derive(Clone, Debug, Default)]
pub struct CancellationHandle(Arc<CancellationState>);

impl CancellationHandle {
    /// Cancels the invocation idempotently and wakes blocked sinks.
    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            self.0.notify.notify_waiters();
        }
    }

    /// Reports whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.0.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

/// Executor-owned bounded sink passed to provider adapters.
#[derive(Clone, Debug)]
pub struct TokenSink {
    sender: mpsc::Sender<Result<Token, ProviderError>>,
    cancellation: CancellationHandle,
}

impl TokenSink {
    async fn send_terminal(&self, event: Result<Token, ProviderError>) {
        let _ = self.sender.send(event).await;
    }

    /// Sends one ordered event while honoring backpressure and cancellation.
    ///
    /// # Errors
    ///
    /// Returns a normalized cancellation error if the invocation was cancelled
    /// or its consumer closed.
    pub async fn send(&self, event: Result<Token, ProviderError>) -> Result<(), ProviderError> {
        if self.cancellation.is_cancelled() {
            return Err(cancelled_error());
        }
        tokio::select! {
            biased;
            () = self.cancellation.cancelled() => Err(cancelled_error()),
            result = self.sender.send(event) => result.map_err(|_| cancelled_error()),
        }
    }
}

/// Backward-compatible adapter sink name.
pub type TokenSender = TokenSink;

/// Bounded token stream returned by the executor.
pub type TokenStream = mpsc::Receiver<Result<Token, ProviderError>>;

/// Invalid streaming executor configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("pipeline channel capacity must be greater than zero")]
pub struct PipelineConfigError;

/// Executes provider calls on Tokio with a bounded token channel.
#[derive(Debug, Clone, Copy)]
pub struct PipelineExecutor {
    channel_capacity: usize,
}

impl PipelineExecutor {
    /// Creates an executor with the maximum buffered token events.
    ///
    /// # Errors
    ///
    /// Returns an error when capacity is zero.
    pub fn new(channel_capacity: usize) -> Result<Self, PipelineConfigError> {
        if channel_capacity == 0 {
            return Err(PipelineConfigError);
        }
        Ok(Self { channel_capacity })
    }

    /// Starts one invocation and returns its bounded token stream.
    #[must_use]
    pub fn execute(
        &self,
        provider: Arc<dyn ProviderAdapter>,
        request: ModelRequest,
    ) -> TokenStream {
        self.execute_cancellable(provider, request).0
    }

    /// Starts one invocation with explicit idempotent cancellation.
    #[must_use]
    pub fn execute_cancellable(
        &self,
        provider: Arc<dyn ProviderAdapter>,
        request: ModelRequest,
    ) -> (TokenStream, CancellationHandle) {
        let (sender, stream) = mpsc::channel(self.channel_capacity);
        let cancellation = CancellationHandle::default();
        let sink = TokenSink {
            sender,
            cancellation: cancellation.clone(),
        };
        let terminal = sink.clone();
        tokio::spawn(async move {
            if let Err(error) = provider.stream(request, sink).await
                && error.category() != ProviderErrorCategory::Cancelled
            {
                terminal.send_terminal(Err(error)).await;
            }
        });
        (stream, cancellation)
    }
}

fn cancelled_error() -> ProviderError {
    ProviderError::categorized(
        ProviderErrorCategory::Cancelled,
        "provider invocation cancelled",
    )
}
