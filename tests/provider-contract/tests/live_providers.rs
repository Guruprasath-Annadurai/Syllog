//! Explicitly opt-in live provider smoke tests for isolated nightly quotas.

use std::sync::Arc;
use std::time::Duration;

use syllog_provider_anthropic::AnthropicAdapter;
use syllog_provider_openai::OpenAiAdapter;
use syllog_proxy::{ModelRequest, ModelRoute, PipelineExecutor, ProviderAdapter, SecretValue};

#[tokio::test]
#[ignore = "requires SYLLOG_LIVE_PROVIDER_TESTS=1 and isolated OpenAI quota"]
async fn openai_live_stream_returns_at_least_one_token() {
    if !live_enabled() {
        return;
    }
    let key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");
    let model =
        std::env::var("SYLLOG_OPENAI_TEST_MODEL").expect("SYLLOG_OPENAI_TEST_MODEL is required");
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(
        OpenAiAdapter::http(
            SecretValue::new(key),
            "https://api.openai.com/v1/chat/completions",
            1,
            Duration::from_secs(30),
        )
        .unwrap(),
    );
    assert_first_token(adapter, model).await;
}

#[tokio::test]
#[ignore = "requires SYLLOG_LIVE_PROVIDER_TESTS=1 and isolated Anthropic quota"]
async fn anthropic_live_stream_returns_at_least_one_token() {
    if !live_enabled() {
        return;
    }
    let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY is required");
    let model = std::env::var("SYLLOG_ANTHROPIC_TEST_MODEL")
        .expect("SYLLOG_ANTHROPIC_TEST_MODEL is required");
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(
        AnthropicAdapter::http(
            SecretValue::new(key),
            "https://api.anthropic.com/v1/messages",
            1,
            Duration::from_secs(30),
        )
        .unwrap(),
    );
    assert_first_token(adapter, model).await;
}

async fn assert_first_token(adapter: Arc<dyn ProviderAdapter>, model: String) {
    let provider = adapter.descriptor().name.clone();
    let request = ModelRequest {
        route: ModelRoute {
            name: "nightly-smoke".into(),
            provider,
            model,
        },
        input: "Reply with the single word OK.".into(),
    };
    let (mut stream, cancellation) = PipelineExecutor::new(1)
        .unwrap()
        .execute_cancellable(adapter, request);
    let token = tokio::time::timeout(Duration::from_secs(30), stream.recv())
        .await
        .expect("live provider timed out")
        .expect("live provider closed without a token")
        .expect("live provider returned an error");
    assert!(!token.text.is_empty());
    cancellation.cancel();
}

fn live_enabled() -> bool {
    std::env::var("SYLLOG_LIVE_PROVIDER_TESTS").as_deref() == Ok("1")
}
