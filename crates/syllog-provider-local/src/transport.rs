//! Real bounded transports for local model processes and sockets.

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use syllog_proxy::{
    FrameTransport, ProviderError, ProviderErrorCategory, TransportFrame, TransportFrameSender,
    TransportFuture, TransportRequest, transport_frame_channel,
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::process::Command;
use tokio::time::{Instant, timeout_at};

const MAX_LOCAL_FRAME_BYTES: usize = 1024 * 1024;

/// Invalid local model transport policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalTransportConfigError {
    /// A required bound or process path is invalid.
    #[error("invalid local model transport: {0}")]
    Invalid(String),
}

/// Explicit executable capability for a newline-delimited local model process.
#[derive(Clone, Debug)]
pub struct LocalProcessTransport {
    program: PathBuf,
    arguments: Vec<OsString>,
    frame_capacity: usize,
    deadline: Duration,
}

impl LocalProcessTransport {
    /// Creates a process transport without invoking a shell.
    ///
    /// # Errors
    ///
    /// Rejects an empty executable path, zero capacity, or zero deadline.
    pub fn new(
        program: impl Into<PathBuf>,
        arguments: impl IntoIterator<Item = impl Into<OsString>>,
        frame_capacity: usize,
        deadline: Duration,
    ) -> Result<Self, LocalTransportConfigError> {
        let program = program.into();
        validate(&program, frame_capacity, deadline)?;
        Ok(Self {
            program,
            arguments: arguments.into_iter().map(Into::into).collect(),
            frame_capacity,
            deadline,
        })
    }
}

impl FrameTransport for LocalProcessTransport {
    fn frames(&self, request: TransportRequest) -> TransportFuture<'_> {
        let transport = self.clone();
        Box::pin(async move { transport.open(request).await })
    }
}

impl LocalProcessTransport {
    async fn open(
        self,
        request: TransportRequest,
    ) -> Result<syllog_proxy::TransportFrameStream, ProviderError> {
        let invocation_deadline = Instant::now() + self.deadline;
        let mut child = Command::new(&self.program)
            .args(&self.arguments)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| unavailable(format!("could not start local model: {error}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| unavailable("local model stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| unavailable("local model stdout is unavailable"))?;
        timeout_at(invocation_deadline, async {
            write_request(&mut stdin, &request).await?;
            stdin
                .shutdown()
                .await
                .map_err(|error| unavailable(format!("could not close local model input: {error}")))
        })
        .await
        .map_err(|_| timed_out("local model invocation exceeded its deadline"))??;
        let (sender, receiver) = transport_frame_channel(self.frame_capacity)?;
        tokio::spawn(async move {
            let operation = async {
                read_frames(stdout, &sender).await?;
                let status = child
                    .wait()
                    .await
                    .map_err(|error| unavailable(format!("could not join local model: {error}")))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(unavailable(format!(
                        "local model exited with status {status}"
                    )))
                }
            };
            send_terminal_error(&sender, timeout_at(invocation_deadline, operation).await).await;
        });
        Ok(receiver)
    }
}

/// Explicit loopback TCP capability for a newline-delimited local model server.
#[derive(Clone, Debug)]
pub struct LocalSocketTransport {
    address: SocketAddr,
    frame_capacity: usize,
    deadline: Duration,
}

impl LocalSocketTransport {
    /// Creates a loopback-only socket transport.
    ///
    /// # Errors
    ///
    /// Rejects non-loopback addresses or zero bounds.
    pub fn new(
        address: SocketAddr,
        frame_capacity: usize,
        deadline: Duration,
    ) -> Result<Self, LocalTransportConfigError> {
        if !address.ip().is_loopback() {
            return Err(LocalTransportConfigError::Invalid(
                "local model socket must use a loopback address".into(),
            ));
        }
        validate_socket(frame_capacity, deadline)?;
        Ok(Self {
            address,
            frame_capacity,
            deadline,
        })
    }
}

impl FrameTransport for LocalSocketTransport {
    fn frames(&self, request: TransportRequest) -> TransportFuture<'_> {
        let transport = self.clone();
        Box::pin(async move { transport.open(request).await })
    }
}

impl LocalSocketTransport {
    async fn open(
        self,
        request: TransportRequest,
    ) -> Result<syllog_proxy::TransportFrameStream, ProviderError> {
        let invocation_deadline = Instant::now() + self.deadline;
        let mut socket = timeout_at(
            invocation_deadline,
            tokio::net::TcpStream::connect(self.address),
        )
        .await
        .map_err(|_| timed_out("local model socket connection timed out"))?
        .map_err(|error| unavailable(format!("could not connect local model: {error}")))?;
        timeout_at(invocation_deadline, write_request(&mut socket, &request))
            .await
            .map_err(|_| timed_out("local model invocation exceeded its deadline"))??;
        let (reader, _) = socket.into_split();
        let (sender, receiver) = transport_frame_channel(self.frame_capacity)?;
        tokio::spawn(async move {
            let operation = read_frames(reader, &sender);
            send_terminal_error(&sender, timeout_at(invocation_deadline, operation).await).await;
        });
        Ok(receiver)
    }
}

async fn write_request(
    writer: &mut (impl AsyncWrite + Unpin),
    request: &TransportRequest,
) -> Result<(), ProviderError> {
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "model": request.model,
        "input": request.input,
        "stream": true
    }))
    .map_err(|error| protocol(format!("could not encode local request: {error}")))?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|error| unavailable(format!("could not write local model request: {error}")))
}

async fn read_frames(
    mut reader: impl AsyncRead + Unpin,
    sender: &TransportFrameSender,
) -> Result<(), ProviderError> {
    let mut pending = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| unavailable(format!("local model stream failed: {error}")))?;
        if read == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..read]);
        if pending.len() > MAX_LOCAL_FRAME_BYTES {
            return Err(protocol("local model frame exceeded one megabyte"));
        }
        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let mut line = pending.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                continue;
            }
            let line =
                String::from_utf8(line).map_err(|_| protocol("local model frame is not UTF-8"))?;
            sender
                .send(TransportFrame::Data(line))
                .await
                .map_err(|_| cancelled())?;
        }
    }
    if !pending.is_empty() {
        let line = String::from_utf8(pending)
            .map_err(|_| protocol("final local model frame is not UTF-8"))?;
        sender
            .send(TransportFrame::Data(line))
            .await
            .map_err(|_| cancelled())?;
    }
    Ok(())
}

async fn send_terminal_error(
    sender: &TransportFrameSender,
    result: Result<Result<(), ProviderError>, tokio::time::error::Elapsed>,
) {
    let error = match result {
        Ok(Ok(())) => return,
        Ok(Err(error)) => error,
        Err(_) => timed_out("local model invocation exceeded its deadline"),
    };
    let _ = sender.send(TransportFrame::Error(error)).await;
}

fn validate(
    program: &Path,
    frame_capacity: usize,
    deadline: Duration,
) -> Result<(), LocalTransportConfigError> {
    if program.as_os_str().is_empty() {
        return Err(LocalTransportConfigError::Invalid(
            "local model executable path is empty".into(),
        ));
    }
    validate_socket(frame_capacity, deadline)
}

fn validate_socket(
    frame_capacity: usize,
    deadline: Duration,
) -> Result<(), LocalTransportConfigError> {
    if frame_capacity == 0 || deadline.is_zero() {
        Err(LocalTransportConfigError::Invalid(
            "frame capacity and deadline must be greater than zero".into(),
        ))
    } else {
        Ok(())
    }
}

fn unavailable(message: impl Into<String>) -> ProviderError {
    ProviderError::categorized(ProviderErrorCategory::Unavailable, message)
}

fn protocol(message: impl Into<String>) -> ProviderError {
    ProviderError::categorized(ProviderErrorCategory::Protocol, message)
}

fn timed_out(message: impl Into<String>) -> ProviderError {
    ProviderError::categorized(ProviderErrorCategory::Timeout, message)
}

fn cancelled() -> ProviderError {
    ProviderError::categorized(
        ProviderErrorCategory::Cancelled,
        "local model consumer closed",
    )
}
