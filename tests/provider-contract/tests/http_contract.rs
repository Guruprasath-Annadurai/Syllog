//! Real loopback HTTP/SSE contracts for remote provider adapters.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Response, StatusCode, header};
use axum::routing::post;
use futures_util::stream;
use syllog_provider_anthropic::AnthropicAdapter;
use syllog_provider_openai::OpenAiAdapter;
use syllog_proxy::{
    ModelRequest, ModelRoute, PipelineExecutor, ProviderAdapter, ProviderErrorCategory,
    SecretValue, Token,
};

#[tokio::test]
async fn openai_and_anthropic_stream_incrementally_with_exact_authentication_headers() {
    let app = Router::new()
        .route("/openai", post(openai))
        .route("/anthropic", post(anthropic));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let openai: std::sync::Arc<dyn ProviderAdapter> = std::sync::Arc::new(
        OpenAiAdapter::http(
            SecretValue::new("openai-key"),
            format!("http://{address}/openai"),
            1,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    assert_incremental(openai).await;

    let anthropic: std::sync::Arc<dyn ProviderAdapter> = std::sync::Arc::new(
        AnthropicAdapter::http(
            SecretValue::new("anthropic-key"),
            format!("http://{address}/anthropic"),
            1,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    assert_incremental(anthropic).await;
    server.abort();
}

#[tokio::test]
async fn rate_limit_status_preserves_retry_after_and_stream_failures_keep_partial_tokens() {
    let app = Router::new()
        .route("/limited", post(rate_limited))
        .route("/broken", post(broken_stream));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let limited: std::sync::Arc<dyn ProviderAdapter> = std::sync::Arc::new(
        OpenAiAdapter::http(
            SecretValue::new("key"),
            format!("http://{address}/limited"),
            1,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let mut stream = PipelineExecutor::new(1)
        .unwrap()
        .execute(limited, request("openai"));
    let error = stream.recv().await.unwrap().unwrap_err();
    assert_eq!(error.category(), ProviderErrorCategory::RateLimited);
    assert_eq!(error.retry_after(), Some(Duration::from_secs(3)));

    let broken: std::sync::Arc<dyn ProviderAdapter> = std::sync::Arc::new(
        OpenAiAdapter::http(
            SecretValue::new("key"),
            format!("http://{address}/broken"),
            1,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let mut stream = PipelineExecutor::new(1)
        .unwrap()
        .execute(broken, request("openai"));
    assert_eq!(stream.recv().await, Some(Ok(Token::from("first"))));
    assert_eq!(
        stream.recv().await.unwrap().unwrap_err().category(),
        ProviderErrorCategory::Unavailable
    );
    server.abort();
}

#[tokio::test]
async fn provider_redirects_are_rejected_without_forwarding_credentials() {
    let redirected = Arc::new(AtomicUsize::new(0));
    let redirected_handler = Arc::clone(&redirected);
    let target = Router::new().route(
        "/target",
        post(move || {
            let redirected_handler = Arc::clone(&redirected_handler);
            async move {
                redirected_handler.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }),
    );
    let (target_url, target_server) = serve(target).await;
    let redirect_url = format!("{target_url}/target");
    let source = Router::new().route(
        "/redirect",
        post(move || async move {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [(header::LOCATION, redirect_url)],
            )
        }),
    );
    let (source_url, source_server) = serve(source).await;
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(
        OpenAiAdapter::http(
            SecretValue::new("redirect-secret"),
            format!("{source_url}/redirect"),
            1,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let mut stream = PipelineExecutor::new(1)
        .unwrap()
        .execute(adapter, request("openai"));
    let error = stream.recv().await.unwrap().unwrap_err();
    assert_eq!(error.category(), ProviderErrorCategory::Protocol);
    assert_eq!(redirected.load(Ordering::SeqCst), 0);
    source_server.abort();
    target_server.abort();
}

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), server)
}

async fn assert_incremental(adapter: std::sync::Arc<dyn ProviderAdapter>) {
    let provider = adapter.descriptor().name.clone();
    let mut tokens = PipelineExecutor::new(1)
        .unwrap()
        .execute(adapter, request(&provider));
    assert_eq!(tokens.recv().await, Some(Ok(Token::from("first"))));
    assert!(
        tokio::time::timeout(Duration::from_millis(15), tokens.recv())
            .await
            .is_err()
    );
    assert_eq!(tokens.recv().await, Some(Ok(Token::from("second"))));
    assert_eq!(tokens.recv().await, None);
}

fn request(provider: &str) -> ModelRequest {
    ModelRequest {
        route: ModelRoute {
            name: "contract".into(),
            provider: provider.into(),
            model: "model-v1".into(),
        },
        input: "private prompt".into(),
    }
}

async fn openai(headers: HeaderMap) -> Response<Body> {
    assert_eq!(
        headers.get(header::AUTHORIZATION).unwrap(),
        "Bearer openai-key"
    );
    timed_sse(
        r#"{"choices":[{"delta":{"content":"first"}}]}"#,
        r#"{"choices":[{"delta":{"content":"second"}}]}"#,
    )
}

async fn anthropic(headers: HeaderMap) -> Response<Body> {
    assert_eq!(headers.get("x-api-key").unwrap(), "anthropic-key");
    assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
    timed_sse(
        r#"{"type":"content_block_delta","delta":{"text":"first"}}"#,
        r#"{"type":"content_block_delta","delta":{"text":"second"}}"#,
    )
}

fn timed_sse(first: &'static str, second: &'static str) -> Response<Body> {
    let frames = stream::unfold(0_u8, move |state| async move {
        match state {
            0 => Some((
                Ok::<_, Infallible>(Bytes::from(format!("data: {first}\n\n"))),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Some((Ok(Bytes::from(format!("data: {second}\n\n"))), 2))
            }
            2 => Some((Ok(Bytes::from_static(b"data: [DONE]\n\n")), 3)),
            _ => None,
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(frames))
        .unwrap()
}

async fn rate_limited() -> Response<Body> {
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(header::RETRY_AFTER, "3")
        .body(Body::from("quota exhausted"))
        .unwrap()
}

async fn broken_stream() -> Response<Body> {
    let frames = stream::unfold(0_u8, |state| async move {
        match state {
            0 => Some((
                Ok::<_, std::io::Error>(Bytes::from_static(
                    b"data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n",
                )),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Some((Err(std::io::Error::other("connection reset")), 2))
            }
            _ => None,
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(frames))
        .unwrap()
}
