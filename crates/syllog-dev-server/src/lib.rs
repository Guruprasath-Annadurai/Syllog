//! Incremental Syllog project development service.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use syllog_compiler::{EditorReport, compile};
use syllog_project::{Project, Target};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Monotonic build identifier within one development session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuildId(pub u64);

/// Content-addressed successful build identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactId {
    /// Target whose output became ready.
    pub target: String,
    /// Lowercase SHA-256 digest of the compiled source input.
    pub digest: String,
}

/// Stable event protocol consumed by terminals and editor integrations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "payload", rename_all = "snake_case")]
pub enum DevEvent {
    /// A debounced build began.
    Building(BuildId),
    /// Complete machine-readable compiler outcome.
    Diagnostics(EditorReport),
    /// A successful build is ready.
    Ready(ArtifactId),
    /// The managed runtime accepted the new successful build.
    RuntimeRestarted,
}

/// Development-loop timing policy.
#[derive(Clone, Copy, Debug)]
pub struct DevOptions {
    /// Source fingerprint polling interval.
    pub poll_interval: Duration,
    /// Quiet period required before rebuilding a changed target.
    pub debounce: Duration,
}

impl Default for DevOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(50),
            debounce: Duration::from_millis(100),
        }
    }
}

/// Observable incremental-work counters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DevStats {
    /// Total target builds executed.
    pub total_builds: u64,
    /// Builds executed per target.
    pub by_target: BTreeMap<String, u64>,
}

/// Development service startup or runtime failure.
#[derive(Debug, Error)]
pub enum DevError {
    /// A project has no source targets.
    #[error("project has no targets")]
    NoTargets,
    /// The background task failed.
    #[error("development task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// Controller and structured event stream for a running development service.
pub struct DevHandle {
    events: mpsc::Receiver<DevEvent>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    stats: Arc<Mutex<DevStats>>,
}

impl DevHandle {
    /// Receives the next structured build event.
    pub async fn next_event(&mut self) -> Option<DevEvent> {
        self.events.recv().await
    }

    /// Returns a consistent snapshot of incremental-work counters.
    #[must_use]
    pub fn stats(&self) -> DevStats {
        self.stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Stops polling and waits for the service task to exit.
    ///
    /// # Errors
    ///
    /// Returns an error if the background task panicked or was cancelled.
    pub async fn shutdown(mut self) -> Result<(), DevError> {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.await?;
        }
        Ok(())
    }
}

/// Starts a debounced, target-incremental project development loop.
///
/// # Errors
///
/// Returns an error when the project has no buildable targets.
pub async fn serve(project: Project, options: DevOptions) -> Result<DevHandle, DevError> {
    if project.manifest.targets.is_empty() {
        return Err(DevError::NoTargets);
    }
    let (event_sender, events) = mpsc::channel(64);
    let (shutdown_sender, shutdown) = oneshot::channel();
    let stats = Arc::new(Mutex::new(DevStats::default()));
    let task_stats = Arc::clone(&stats);
    let targets = project.manifest.targets;
    let task = tokio::spawn(async move {
        run_loop(targets, options, event_sender, shutdown, task_stats).await;
    });
    tokio::task::yield_now().await;
    Ok(DevHandle {
        events,
        shutdown: Some(shutdown_sender),
        task: Some(task),
        stats,
    })
}

async fn run_loop(
    targets: Vec<Target>,
    options: DevOptions,
    sender: mpsc::Sender<DevEvent>,
    mut shutdown: oneshot::Receiver<()>,
    stats: Arc<Mutex<DevStats>>,
) {
    let mut interval = tokio::time::interval(options.poll_interval);
    let mut observed = BTreeMap::<PathBuf, Option<[u8; 32]>>::new();
    let mut pending = BTreeMap::<PathBuf, Instant>::new();
    let mut next_build = 1_u64;
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = interval.tick() => {
                observe_changes(&targets, &mut observed, &mut pending);
                let ready = pending
                    .iter()
                    .filter(|(_, changed)| changed.elapsed() >= options.debounce)
                    .map(|(path, _)| path.clone())
                    .collect::<Vec<_>>();
                for path in ready {
                    pending.remove(&path);
                    let Some(target) = targets.iter().find(|target| target.path == path) else {
                        continue;
                    };
                    if !build_target(target, BuildId(next_build), &sender, &stats).await {
                        return;
                    }
                    next_build = next_build.saturating_add(1);
                }
            }
        }
    }
}

fn observe_changes(
    targets: &[Target],
    observed: &mut BTreeMap<PathBuf, Option<[u8; 32]>>,
    pending: &mut BTreeMap<PathBuf, Instant>,
) {
    for target in targets {
        let fingerprint = source_fingerprint(&target.path);
        if observed.get(&target.path) != Some(&fingerprint) {
            observed.insert(target.path.clone(), fingerprint);
            pending.insert(target.path.clone(), Instant::now());
        }
    }
}

async fn build_target(
    target: &Target,
    build_id: BuildId,
    sender: &mpsc::Sender<DevEvent>,
    stats: &Mutex<DevStats>,
) -> bool {
    {
        let mut current = stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        current.total_builds = current.total_builds.saturating_add(1);
        *current.by_target.entry(target.name.clone()).or_default() += 1;
    }
    if sender.send(DevEvent::Building(build_id)).await.is_err() {
        return false;
    }
    let source = std::fs::read_to_string(&target.path).unwrap_or_default();
    let compilation = compile(target.path.display().to_string(), &source);
    let succeeded = compilation.success();
    if sender
        .send(DevEvent::Diagnostics(EditorReport::from(&compilation)))
        .await
        .is_err()
    {
        return false;
    }
    if succeeded {
        let artifact = ArtifactId {
            target: target.name.clone(),
            digest: format!("{:x}", Sha256::digest(source.as_bytes())),
        };
        if sender.send(DevEvent::Ready(artifact)).await.is_err()
            || sender.send(DevEvent::RuntimeRestarted).await.is_err()
        {
            return false;
        }
    }
    true
}

fn source_fingerprint(path: &Path) -> Option<[u8; 32]> {
    std::fs::read(path)
        .ok()
        .map(|bytes| Sha256::digest(bytes).into())
}
