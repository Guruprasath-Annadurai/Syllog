//! Local-model streaming-frame adapter for the Syllog provider ABI.

use std::sync::Arc;

use serde_json::Value;
use syllog_proxy::{
    CredentialKind, FrameTransport, ModelRequest, ProviderAbiVersion, ProviderAdapter,
    ProviderDescriptor, ProviderError, ProviderErrorCategory, ProviderFuture, Token, TokenSink,
    TransportFrame, TransportRequest,
};

/// Local model adapter using an injected process or socket transport.
pub struct LocalModelAdapter {
    descriptor: ProviderDescriptor,
    transport: Arc<dyn FrameTransport>,
}

impl LocalModelAdapter {
    /// Creates a local-model adapter using ABI version 1.
    #[must_use]
    pub fn new(transport: Arc<dyn FrameTransport>) -> Self {
        Self {
            descriptor: ProviderDescriptor {
                name: "local".into(),
                abi: ProviderAbiVersion { major: 1, minor: 0 },
                credentials: CredentialKind::LocalCapability,
            },
            transport,
        }
    }
}

impl ProviderAdapter for LocalModelAdapter {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    fn stream(&self, request: ModelRequest, sink: TokenSink) -> ProviderFuture<'_> {
        Box::pin(async move {
            let frames = self
                .transport
                .frames(TransportRequest {
                    provider: self.descriptor.name.clone(),
                    model: request.route.model,
                    input: request.input,
                    authorization: None,
                })
                .await?;
            for frame in frames {
                match frame {
                    TransportFrame::Data(data) => {
                        let value: Value = serde_json::from_str(&data).map_err(protocol_error)?;
                        let text = value
                            .get("token")
                            .and_then(Value::as_str)
                            .ok_or_else(|| protocol_error("missing token"))?;
                        sink.send(Ok(Token::from(text))).await?;
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
        format!("invalid local-model stream frame: {error}"),
    )
}
