//! `Anthropic` streaming-frame adapter for the Syllog provider ABI.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use syllog_proxy::{
    CredentialKind, FrameTransport, HttpSseTransport, HttpTransportConfigError, ModelRequest,
    ProviderAbiVersion, ProviderAdapter, ProviderDescriptor, ProviderError, ProviderErrorCategory,
    ProviderFuture, SecretValue, Token, TokenSink, TransportFrame, TransportRequest,
};

/// `Anthropic` provider adapter with an injected transport and bearer capability.
pub struct AnthropicAdapter {
    descriptor: ProviderDescriptor,
    credential: SecretValue,
    transport: Arc<dyn FrameTransport>,
}

impl AnthropicAdapter {
    /// Creates an `Anthropic` adapter using ABI version 1.
    #[must_use]
    pub fn new(credential: SecretValue, transport: Arc<dyn FrameTransport>) -> Self {
        Self {
            descriptor: ProviderDescriptor {
                name: "anthropic".into(),
                abi: ProviderAbiVersion { major: 1, minor: 0 },
                credentials: CredentialKind::BearerToken,
            },
            credential,
            transport,
        }
    }

    /// Creates an authenticated incremental HTTP/SSE `Anthropic` adapter.
    ///
    /// # Errors
    ///
    /// Rejects unsafe endpoints or invalid transport limits.
    pub fn http(
        credential: SecretValue,
        endpoint: impl Into<String>,
        frame_capacity: usize,
        deadline: Duration,
    ) -> Result<Self, HttpTransportConfigError> {
        let transport = Arc::new(HttpSseTransport::new(endpoint, frame_capacity, deadline)?);
        Ok(Self::new(credential, transport))
    }
}

impl ProviderAdapter for AnthropicAdapter {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    fn stream(&self, request: ModelRequest, sink: TokenSink) -> ProviderFuture<'_> {
        Box::pin(async move {
            let mut frames = self
                .transport
                .frames(TransportRequest {
                    provider: self.descriptor.name.clone(),
                    model: request.route.model,
                    input: request.input,
                    authorization: Some(self.credential.clone()),
                })
                .await?;
            while let Some(frame) = frames.recv().await {
                match frame {
                    TransportFrame::Data(data) => {
                        let value: Value = serde_json::from_str(&data).map_err(protocol_error)?;
                        if let Some(text) = value.pointer("/delta/text").and_then(Value::as_str) {
                            sink.send(Ok(Token::from(text))).await?;
                        }
                    }
                    TransportFrame::Error(error) => return Err(error),
                }
            }
            Ok(())
        })
    }
}

fn protocol_error(error: impl std::fmt::Display) -> ProviderError {
    ProviderError::categorized(
        ProviderErrorCategory::Protocol,
        format!("invalid Anthropic stream frame: {error}"),
    )
}
