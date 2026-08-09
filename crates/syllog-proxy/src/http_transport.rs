//! Bounded incremental HTTP/SSE transport for remote model providers.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::StreamExt as _;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue, RETRY_AFTER};
use serde_json::json;

use crate::{
    FrameTransport, ProviderError, ProviderErrorCategory, TransportFrame, TransportFuture,
    TransportRequest, transport_frame_channel,
};

const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 4096;

/// Invalid remote transport configuration.
#[derive(Debug, thiserror::Error)]
pub enum HttpTransportConfigError {
    /// Endpoint URL or policy is invalid.
    #[error("invalid HTTP provider transport: {0}")]
    Invalid(String),
    /// HTTP client construction failed.
    #[error("could not create HTTP provider client: {0}")]
    Client(#[from] reqwest::Error),
}

/// Provider-aware HTTPS client that incrementally decodes bounded SSE events.
#[derive(Clone)]
pub struct HttpSseTransport {
    endpoint: Arc<str>,
    frame_capacity: usize,
    client: reqwest::Client,
}

impl HttpSseTransport {
    /// Creates a no-proxy HTTPS transport. Loopback HTTP is allowed for contracts.
    ///
    /// # Errors
    ///
    /// Rejects unsafe endpoints, zero limits, and invalid HTTP configuration.
    pub fn new(
        endpoint: impl Into<String>,
        frame_capacity: usize,
        deadline: Duration,
    ) -> Result<Self, HttpTransportConfigError> {
        if frame_capacity == 0 || deadline.is_zero() {
            return Err(HttpTransportConfigError::Invalid(
                "frame capacity and deadline must be greater than zero".into(),
            ));
        }
        let endpoint = endpoint.into();
        let parsed = reqwest::Url::parse(&endpoint)
            .map_err(|error| HttpTransportConfigError::Invalid(error.to_string()))?;
        let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if parsed.scheme() != "https" && !loopback {
            return Err(HttpTransportConfigError::Invalid(
                "provider endpoint must use HTTPS except on loopback".into(),
            ));
        }
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(HttpTransportConfigError::Invalid(
                "provider endpoint must not contain credentials or a fragment".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(deadline)
            .build()?;
        Ok(Self {
            endpoint: Arc::from(endpoint),
            frame_capacity,
            client,
        })
    }
}

impl FrameTransport for HttpSseTransport {
    fn frames(&self, request: TransportRequest) -> TransportFuture<'_> {
        let client = self.client.clone();
        let endpoint = Arc::clone(&self.endpoint);
        let capacity = self.frame_capacity;
        Box::pin(open_http_stream(client, endpoint, capacity, request))
    }
}

async fn open_http_stream(
    client: reqwest::Client,
    endpoint: Arc<str>,
    capacity: usize,
    request: TransportRequest,
) -> Result<crate::TransportFrameStream, ProviderError> {
    let response = build_request(&client, endpoint.as_ref(), request)?
        .send()
        .await
        .map_err(|error| map_reqwest(&error))?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type.starts_with("text/event-stream") {
        return Err(protocol("provider response is not text/event-stream"));
    }
    let (sender, receiver) = transport_frame_channel(capacity)?;
    tokio::spawn(decode_response(response, sender));
    Ok(receiver)
}

fn build_request(
    client: &reqwest::Client,
    endpoint: &str,
    request: TransportRequest,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    let builder = client.post(endpoint).header("accept", "text/event-stream");
    match request.provider.as_str() {
        "openai" => openai_request(builder, request),
        "anthropic" => anthropic_request(builder, request),
        provider => Err(protocol(format!(
            "HTTP/SSE transport does not support provider '{provider}'"
        ))),
    }
}

fn openai_request(
    builder: reqwest::RequestBuilder,
    request: TransportRequest,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    let credential = request.authorization.ok_or_else(|| {
        ProviderError::categorized(
            ProviderErrorCategory::Authentication,
            "OpenAI transport requires a bearer credential",
        )
    })?;
    let header =
        credential.with_exposed(|secret| HeaderValue::from_str(&format!("Bearer {secret}")));
    let mut header = sensitive_header(header, "OpenAI")?;
    header.set_sensitive(true);
    Ok(builder.header(AUTHORIZATION, header).json(&json!({
        "model": request.model,
        "stream": true,
        "messages": [{"role": "user", "content": request.input}]
    })))
}

fn anthropic_request(
    builder: reqwest::RequestBuilder,
    request: TransportRequest,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    let credential = request.authorization.ok_or_else(|| {
        ProviderError::categorized(
            ProviderErrorCategory::Authentication,
            "Anthropic transport requires an API key",
        )
    })?;
    let mut header = sensitive_header(credential.with_exposed(HeaderValue::from_str), "Anthropic")?;
    header.set_sensitive(true);
    Ok(builder
        .header("x-api-key", header)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": request.model,
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": request.input}]
        })))
}

fn sensitive_header(
    header: Result<HeaderValue, reqwest::header::InvalidHeaderValue>,
    provider: &str,
) -> Result<HeaderValue, ProviderError> {
    header.map_err(|_| {
        ProviderError::categorized(
            ProviderErrorCategory::Authentication,
            format!("{provider} credential is not valid HTTP header data"),
        )
    })
}

async fn decode_response(response: reqwest::Response, sender: crate::TransportFrameSender) {
    let mut decoder = SseDecoder::default();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let events = match chunk
            .map_err(|error| map_reqwest(&error))
            .and_then(|chunk| decoder.push(&chunk))
        {
            Ok(events) => events,
            Err(error) => {
                let _ = sender.send(TransportFrame::Error(error)).await;
                return;
            }
        };
        for event in events {
            if event == "[DONE]" || sender.send(TransportFrame::Data(event)).await.is_err() {
                return;
            }
        }
    }
    match decoder.finish() {
        Ok(Some(event)) if event != "[DONE]" => {
            let _ = sender.send(TransportFrame::Data(event)).await;
        }
        Err(error) => {
            let _ = sender.send(TransportFrame::Error(error)).await;
        }
        _ => {}
    }
}

#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    data_bytes: usize,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ProviderError> {
        self.pending.extend_from_slice(chunk);
        if self.pending.len() > MAX_SSE_EVENT_BYTES {
            return Err(protocol("SSE event exceeded the one-megabyte limit"));
        }
        let mut events = Vec::new();
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = std::str::from_utf8(&line).map_err(|_| protocol("SSE line is not UTF-8"))?;
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                    self.data_bytes = 0;
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                self.push_data(value.strip_prefix(' ').unwrap_or(value))?;
            }
        }
        Ok(events)
    }

    fn finish(mut self) -> Result<Option<String>, ProviderError> {
        if !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            let line = std::str::from_utf8(&pending)
                .map_err(|_| protocol("final SSE line is not UTF-8"))?;
            if let Some(value) = line.trim_end_matches('\r').strip_prefix("data:") {
                self.push_data(value.strip_prefix(' ').unwrap_or(value))?;
            }
        }
        Ok((!self.data.is_empty()).then(|| self.data.join("\n")))
    }

    fn push_data(&mut self, value: &str) -> Result<(), ProviderError> {
        let separator = usize::from(!self.data.is_empty());
        self.data_bytes = self
            .data_bytes
            .checked_add(value.len() + separator)
            .ok_or_else(|| protocol("SSE event size overflowed"))?;
        if self.data_bytes > MAX_SSE_EVENT_BYTES {
            return Err(protocol("SSE event exceeded the one-megabyte limit"));
        }
        self.data.push(value.to_owned());
        Ok(())
    }
}

async fn response_error(response: reqwest::Response) -> ProviderError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    let mut body = response.bytes_stream();
    let mut bytes = Vec::with_capacity(MAX_ERROR_BODY_BYTES);
    while bytes.len() < MAX_ERROR_BODY_BYTES {
        let Some(Ok(chunk)) = body.next().await else {
            break;
        };
        let remaining = MAX_ERROR_BODY_BYTES - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let message = String::from_utf8_lossy(&bytes);
    let summary = format!("provider returned HTTP {}: {message}", status.as_u16());
    match status.as_u16() {
        401 | 403 => ProviderError::categorized(ProviderErrorCategory::Authentication, summary),
        429 => ProviderError::rate_limited(summary, retry_after),
        500..=599 => ProviderError::categorized(ProviderErrorCategory::Unavailable, summary),
        _ => ProviderError::categorized(ProviderErrorCategory::Protocol, summary),
    }
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let deadline = httpdate::parse_http_date(value).ok()?;
    deadline.duration_since(SystemTime::now()).ok()
}

fn map_reqwest(error: &reqwest::Error) -> ProviderError {
    let category = if error.is_timeout() {
        ProviderErrorCategory::Timeout
    } else {
        ProviderErrorCategory::Unavailable
    };
    ProviderError::categorized(category, format!("provider transport failed: {error}"))
}

fn protocol(message: impl Into<String>) -> ProviderError {
    ProviderError::categorized(ProviderErrorCategory::Protocol, message)
}

#[cfg(test)]
mod tests {
    use super::{MAX_SSE_EVENT_BYTES, SseDecoder};

    #[test]
    fn rejects_aggregate_multiline_events_over_the_limit() {
        let mut decoder = SseDecoder::default();
        let half = "a".repeat(MAX_SSE_EVENT_BYTES / 2);
        decoder.push(format!("data: {half}\n").as_bytes()).unwrap();
        let error = decoder
            .push(format!("data: {half}\n").as_bytes())
            .unwrap_err();
        assert!(error.to_string().contains("one-megabyte"));
    }
}
