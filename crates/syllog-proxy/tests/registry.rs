//! Versioned provider registry and credential safety contracts.

use std::sync::Arc;

use syllog_proxy::{
    CredentialKind, MockProvider, ModelRequest, ModelRoute, PipelineExecutor, ProviderAbiVersion,
    ProviderDescriptor, ProviderLookupError, ProviderRegistry, SecretValue,
};

fn descriptor(name: &str, major: u16, minor: u16) -> ProviderDescriptor {
    ProviderDescriptor {
        name: name.into(),
        abi: ProviderAbiVersion { major, minor },
        credentials: CredentialKind::BearerToken,
    }
}

fn route(provider: &str) -> ModelRoute {
    ModelRoute {
        name: "primary".into(),
        provider: provider.into(),
        model: "model-v1".into(),
    }
}

#[test]
fn registry_rejects_incompatible_abi_before_publishing_snapshot() {
    let mut registry = ProviderRegistry::new(ProviderAbiVersion { major: 1, minor: 2 });
    let adapter = Arc::new(MockProvider::with_descriptor(
        descriptor("future", 2, 0),
        ["unused"],
    ));
    assert!(matches!(
        registry.register(adapter),
        Err(ProviderLookupError::AbiMismatch { .. })
    ));
    assert_eq!(registry.len(), 0);
}

#[test]
fn immutable_snapshot_rejects_duplicates_and_resolves_exact_provider() {
    let mut builder = ProviderRegistry::new(ProviderAbiVersion { major: 1, minor: 2 });
    let first = Arc::new(MockProvider::with_descriptor(
        descriptor("mock", 1, 0),
        ["one"],
    ));
    builder.register(first).unwrap();
    let duplicate = Arc::new(MockProvider::with_descriptor(
        descriptor("mock", 1, 1),
        ["two"],
    ));
    assert!(matches!(
        builder.register(duplicate),
        Err(ProviderLookupError::Duplicate { ref provider }) if provider == "mock"
    ));

    let snapshot = builder.snapshot();
    assert_eq!(
        snapshot.resolve(&route("mock")).unwrap().descriptor().name,
        "mock"
    );
    assert!(matches!(
        snapshot.resolve(&route("missing")),
        Err(ProviderLookupError::Unknown { ref provider }) if provider == "missing"
    ));
}

#[test]
fn secret_values_are_redacted_from_debug_display_and_json() {
    let secret = SecretValue::new("sk-live-do-not-leak");
    for rendered in [
        format!("{secret:?}"),
        secret.to_string(),
        serde_json::to_string(&secret).unwrap(),
    ] {
        assert!(!rendered.contains("sk-live-do-not-leak"));
        assert!(rendered.contains("REDACTED"));
    }
}

#[tokio::test]
async fn explicit_cancellation_closes_stream_without_post_cancel_tokens() {
    let provider = Arc::new(MockProvider::with_descriptor(
        descriptor("mock", 1, 0),
        ["first", "second", "third"],
    ));
    let executor = PipelineExecutor::new(1).unwrap();
    let request = ModelRequest {
        route: route("mock"),
        input: "hello".into(),
    };
    let (mut stream, cancellation) = executor.execute_cancellable(provider, request);
    assert_eq!(stream.recv().await.unwrap().unwrap().text, "first");
    cancellation.cancel();
    assert_eq!(stream.recv().await, None);
}
