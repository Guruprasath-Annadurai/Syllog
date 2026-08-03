//! Identical behavior contracts for all built-in adapters.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use syllog_provider_anthropic::AnthropicAdapter;
use syllog_provider_local::LocalModelAdapter;
use syllog_provider_openai::OpenAiAdapter;
use syllog_proxy::{
    FrameTransport, ModelRequest, ModelRoute, PipelineExecutor, ProviderAdapter, ProviderError,
    ProviderErrorCategory, SecretValue, Token, TransportFrame, TransportFuture, TransportRequest,
};

#[derive(Clone)]
struct ScriptTransport {
    result: Result<Vec<TransportFrame>, ProviderError>,
}

impl FrameTransport for ScriptTransport {
    fn frames(&self, _request: TransportRequest) -> TransportFuture<'_> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

#[derive(Clone, Default)]
struct RecordingTransport {
    requests: Arc<Mutex<Vec<TransportRequest>>>,
}

impl FrameTransport for RecordingTransport {
    fn frames(&self, request: TransportRequest) -> TransportFuture<'_> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[derive(Clone, Copy)]
enum Kind {
    OpenAi,
    Anthropic,
    Local,
}

fn adapter(
    kind: Kind,
    frames: Result<Vec<TransportFrame>, ProviderError>,
) -> Arc<dyn ProviderAdapter> {
    let transport = Arc::new(ScriptTransport { result: frames });
    match kind {
        Kind::OpenAi => Arc::new(OpenAiAdapter::new(
            SecretValue::new("openai-secret"),
            transport,
        )),
        Kind::Anthropic => Arc::new(AnthropicAdapter::new(
            SecretValue::new("anthropic-secret"),
            transport,
        )),
        Kind::Local => Arc::new(LocalModelAdapter::new(transport)),
    }
}

fn valid_frame(kind: Kind, text: &str) -> String {
    match kind {
        Kind::OpenAi => format!(r#"{{"choices":[{{"delta":{{"content":"{text}"}}}}]}}"#),
        Kind::Anthropic => {
            format!(r#"{{"type":"content_block_delta","delta":{{"text":"{text}"}}}}"#)
        }
        Kind::Local => format!(r#"{{"token":"{text}"}}"#),
    }
}

async fn collect(adapter: Arc<dyn ProviderAdapter>) -> Vec<Result<Token, ProviderError>> {
    let request = ModelRequest {
        route: ModelRoute {
            name: "contract".into(),
            provider: adapter.descriptor().name.clone(),
            model: "model-v1".into(),
        },
        input: "private-input".into(),
    };
    let mut stream = PipelineExecutor::new(1).unwrap().execute(adapter, request);
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn all_adapters_preserve_token_order_and_partial_terminal_failure() {
    for kind in [Kind::OpenAi, Kind::Anthropic, Kind::Local] {
        let frames = vec![
            TransportFrame::Data(valid_frame(kind, "a")),
            TransportFrame::Data(valid_frame(kind, "b")),
            TransportFrame::Error(ProviderError::new("upstream reset")),
        ];
        let events = collect(adapter(kind, Ok(frames))).await;
        assert_eq!(events[0], Ok(Token::from("a")));
        assert_eq!(events[1], Ok(Token::from("b")));
        assert_eq!(
            events[2].as_ref().unwrap_err().category(),
            ProviderErrorCategory::Unavailable
        );
        assert_eq!(events.len(), 3);
    }
}

#[tokio::test]
async fn all_adapters_normalize_malformed_rate_limit_and_timeout_failures() {
    for kind in [Kind::OpenAi, Kind::Anthropic, Kind::Local] {
        let malformed = collect(adapter(
            kind,
            Ok(vec![TransportFrame::Data("not-json".into())]),
        ))
        .await;
        assert_eq!(
            malformed[0].as_ref().unwrap_err().category(),
            ProviderErrorCategory::Protocol
        );
    }
    for category in [
        ProviderErrorCategory::RateLimited,
        ProviderErrorCategory::Timeout,
    ] {
        let events = collect(adapter(
            Kind::OpenAi,
            Err(ProviderError::categorized(category, "transport failure")),
        ))
        .await;
        assert_eq!(events[0].as_ref().unwrap_err().category(), category);
        assert_eq!(events.len(), 1);
    }
}

#[tokio::test]
async fn remote_credentials_and_prompts_are_redacted_at_transport_boundary() {
    let transport = Arc::new(RecordingTransport::default());
    let request = ModelRequest {
        route: ModelRoute {
            name: "secure-route".into(),
            provider: "openai".into(),
            model: "model-v1".into(),
        },
        input: "prompt-that-must-not-leak".into(),
    };
    collect_with_request(
        Arc::new(OpenAiAdapter::new(
            SecretValue::new("credential-that-must-not-leak"),
            transport.clone(),
        )),
        request,
    )
    .await;

    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].authorization.is_some());
    let rendered = format!("{:?}", requests[0]);
    assert!(!rendered.contains("credential-that-must-not-leak"));
    assert!(!rendered.contains("prompt-that-must-not-leak"));
    assert!(rendered.contains("[REDACTED]"));
}

#[tokio::test]
async fn every_adapter_honors_cancellation_while_backpressured() {
    for kind in [Kind::OpenAi, Kind::Anthropic, Kind::Local] {
        let frames = (0..32)
            .map(|index| TransportFrame::Data(valid_frame(kind, &index.to_string())))
            .collect();
        let provider = adapter(kind, Ok(frames));
        let request = ModelRequest {
            route: ModelRoute {
                name: "cancel-route".into(),
                provider: provider.descriptor().name.clone(),
                model: "model-v1".into(),
            },
            input: "input".into(),
        };
        let (mut stream, cancellation) = PipelineExecutor::new(1)
            .unwrap()
            .execute_cancellable(provider, request);
        tokio::task::yield_now().await;
        cancellation.cancel();

        let mut buffered = 0;
        while let Some(event) = tokio::time::timeout(Duration::from_secs(1), stream.recv())
            .await
            .expect("cancelled adapter must terminate")
        {
            assert!(event.is_ok());
            buffered += 1;
        }
        assert!(buffered <= 1, "bounded sink leaked {buffered} events");
    }
}

async fn collect_with_request(
    adapter: Arc<dyn ProviderAdapter>,
    request: ModelRequest,
) -> Vec<Result<Token, ProviderError>> {
    let mut stream = PipelineExecutor::new(1).unwrap().execute(adapter, request);
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }
    events
}
