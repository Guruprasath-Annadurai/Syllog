//! Runtime-facing provider and streaming pipeline contracts.

use std::sync::Arc;
use syllog_proxy::{
    MockProvider, ModelRequest, ModelRoute, PipelineExecutor, ProviderAdapter, ProviderError,
    ProviderFuture, Token, TokenSender,
};
use tokio::sync::Notify;
use tokio::time::{Duration, timeout};

fn request() -> ModelRequest {
    ModelRequest {
        route: ModelRoute {
            name: "test-route".into(),
            provider: "mock".into(),
            model: "mock-v1".into(),
        },
        input: "hello".into(),
    }
}

#[tokio::test]
async fn executor_streams_mock_tokens_in_order_and_closes() {
    let provider = Arc::new(MockProvider::tokens(["hel", "lo", "!"]));
    let executor = PipelineExecutor::new(2).expect("capacity is non-zero");
    let mut stream = executor.execute(provider, request());
    let mut received = Vec::new();

    while let Some(event) = stream.recv().await {
        received.push(event.expect("mock stream should succeed"));
    }

    assert_eq!(
        received,
        [Token::from("hel"), Token::from("lo"), Token::from("!")]
    );
}

#[tokio::test]
async fn provider_failure_is_an_ordered_stream_event() {
    let provider = Arc::new(MockProvider::scripted([
        Ok(Token::from("partial")),
        Err(ProviderError::new("upstream disconnected")),
    ]));
    let executor = PipelineExecutor::new(1).expect("capacity is non-zero");
    let mut stream = executor.execute(provider, request());

    assert_eq!(stream.recv().await, Some(Ok(Token::from("partial"))));
    assert_eq!(
        stream.recv().await,
        Some(Err(ProviderError::new("upstream disconnected")))
    );
    assert_eq!(stream.recv().await, None);
}

struct BurstProvider {
    second_send_started: Arc<Notify>,
    second_send_completed: Arc<Notify>,
}

impl ProviderAdapter for BurstProvider {
    fn stream(&self, _request: ModelRequest, output: TokenSender) -> ProviderFuture<'_> {
        Box::pin(async move {
            output
                .send(Ok(Token::from("first")))
                .await
                .map_err(|_| ProviderError::new("consumer closed"))?;
            self.second_send_started.notify_one();
            output
                .send(Ok(Token::from("second")))
                .await
                .map_err(|_| ProviderError::new("consumer closed"))?;
            self.second_send_completed.notify_one();
            Ok(())
        })
    }
}

#[tokio::test]
async fn bounded_stream_applies_backpressure_to_provider() {
    let second_send_started = Arc::new(Notify::new());
    let second_send_completed = Arc::new(Notify::new());
    let provider = Arc::new(BurstProvider {
        second_send_started: Arc::clone(&second_send_started),
        second_send_completed: Arc::clone(&second_send_completed),
    });
    let executor = PipelineExecutor::new(1).expect("capacity is non-zero");
    let mut stream = executor.execute(provider, request());

    second_send_started.notified().await;
    assert!(
        timeout(Duration::from_millis(25), second_send_completed.notified())
            .await
            .is_err(),
        "second token must remain blocked while the one-slot stream is full"
    );
    assert_eq!(stream.recv().await, Some(Ok(Token::from("first"))));
    timeout(Duration::from_secs(1), second_send_completed.notified())
        .await
        .expect("second send should complete after capacity is released");
    assert_eq!(stream.recv().await, Some(Ok(Token::from("second"))));
}
