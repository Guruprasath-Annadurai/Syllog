//! Real local process and loopback socket transport contracts.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use syllog_provider_local::{LocalModelAdapter, LocalProcessTransport, LocalSocketTransport};
use syllog_proxy::{
    ModelRequest, ModelRoute, PipelineExecutor, ProviderAdapter, ProviderErrorCategory, Token,
};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

#[tokio::test]
async fn local_process_transport_streams_incremental_newline_frames_without_a_shell() {
    let transport = LocalProcessTransport::new(
        env!("CARGO_BIN_EXE_local_model_fixture"),
        Vec::<String>::new(),
        1,
        Duration::from_secs(2),
    )
    .unwrap();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(LocalModelAdapter::process(transport));
    let mut tokens = PipelineExecutor::new(1)
        .unwrap()
        .execute(adapter, request());
    assert_eq!(tokens.recv().await, Some(Ok(Token::from("process-first"))));
    assert_eq!(tokens.recv().await, Some(Ok(Token::from("process-second"))));
    assert_eq!(tokens.recv().await, None);
}

#[tokio::test]
async fn local_socket_transport_is_loopback_only_and_streams_real_socket_frames() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let mut request = String::new();
        BufReader::new(reader)
            .read_line(&mut request)
            .await
            .unwrap();
        assert!(request.contains("private-input"));
        writer
            .write_all(b"{\"token\":\"socket-first\"}\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        writer
            .write_all(b"{\"token\":\"socket-second\"}\n")
            .await
            .unwrap();
    });
    let transport = LocalSocketTransport::new(address, 1, Duration::from_secs(2)).unwrap();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(LocalModelAdapter::socket(transport));
    let mut tokens = PipelineExecutor::new(1)
        .unwrap()
        .execute(adapter, request());
    assert_eq!(tokens.recv().await, Some(Ok(Token::from("socket-first"))));
    assert_eq!(tokens.recv().await, Some(Ok(Token::from("socket-second"))));
    assert_eq!(tokens.recv().await, None);
    server.await.unwrap();

    let public = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 80);
    assert!(LocalSocketTransport::new(public, 1, Duration::from_secs(1)).is_err());
}

#[tokio::test]
async fn local_socket_deadline_covers_the_complete_invocation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut request = String::new();
        let mut socket = BufReader::new(socket);
        socket.read_line(&mut request).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop(socket);
    });
    let transport = LocalSocketTransport::new(address, 1, Duration::from_millis(40)).unwrap();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(LocalModelAdapter::socket(transport));
    let mut tokens = PipelineExecutor::new(1)
        .unwrap()
        .execute(adapter, request());
    let error = tokio::time::timeout(Duration::from_millis(250), tokens.recv())
        .await
        .expect("transport deadline must terminate the invocation")
        .expect("transport must return its terminal timeout")
        .expect_err("stalled local model must not succeed");
    assert_eq!(error.category(), ProviderErrorCategory::Timeout);
    server.abort();
}

fn request() -> ModelRequest {
    ModelRequest {
        route: ModelRoute {
            name: "local-contract".into(),
            provider: "local".into(),
            model: "local-model".into(),
        },
        input: "private-input".into(),
    }
}
