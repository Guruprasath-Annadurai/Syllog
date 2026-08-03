//! Structured, bounded, observable production pipeline execution.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{Mutex, Notify, Semaphore};
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Duration, Instant, timeout};

/// Boxed asynchronous stage operation.
pub type StageFuture = Pin<Box<dyn Future<Output = Result<String, StageError>> + Send + 'static>>;
type BranchOutput = (usize, String, Vec<PipelineEvent>);
type JoinedBranch = Result<Result<BranchOutput, StageError>, JoinError>;

trait StageOperation: Send + Sync {
    fn call(&self, context: StageContext, input: String) -> StageFuture;
}

impl<F, Fut> StageOperation for F
where
    F: Fn(StageContext, String) -> Fut + Send + Sync,
    Fut: Future<Output = Result<String, StageError>> + Send + 'static,
{
    fn call(&self, context: StageContext, input: String) -> StageFuture {
        Box::pin(self(context, input))
    }
}

/// Retry behavior for one pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts including the first call.
    pub max_attempts: u32,
    /// Deterministic delay between attempts.
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff: Duration::ZERO,
        }
    }
}

/// Circuit-breaker behavior shared by stage name across executions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitPolicy {
    /// Consecutive terminal failures that open the circuit.
    pub failure_threshold: u32,
    /// Time after opening before one new invocation may try again.
    pub reset_after: Duration,
}

/// Deterministic fan-out result merge policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPolicy {
    /// Results follow branch declaration order regardless of completion order.
    PreserveInputOrder,
    /// Results follow observed completion order.
    CompletionOrder,
}

/// One named executable pipeline stage and its resilience policy.
#[derive(Clone)]
pub struct Stage {
    name: String,
    operation: Arc<dyn StageOperation>,
    retry: RetryPolicy,
    deadline: Option<Duration>,
    circuit: Option<CircuitPolicy>,
}

impl Stage {
    /// Creates a named stage from an owned-input asynchronous operation.
    #[must_use]
    pub fn new<N, F, Fut>(name: N, operation: F) -> Self
    where
        N: Into<String>,
        F: Fn(StageContext, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, StageError>> + Send + 'static,
    {
        Self {
            name: name.into(),
            operation: Arc::new(operation),
            retry: RetryPolicy::default(),
            deadline: None,
            circuit: None,
        }
    }

    /// Applies retry policy. Zero attempts are normalized to one terminal attempt.
    #[must_use]
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry = RetryPolicy {
            max_attempts: policy.max_attempts.max(1),
            backoff: policy.backoff,
        };
        self
    }

    /// Applies a total stage deadline.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Applies a shared circuit policy. Zero threshold is normalized to one.
    #[must_use]
    pub fn with_circuit(mut self, policy: CircuitPolicy) -> Self {
        self.circuit = Some(CircuitPolicy {
            failure_threshold: policy.failure_threshold.max(1),
            reset_after: policy.reset_after,
        });
        self
    }
}

enum StageGroup {
    Serial(Vec<Stage>),
    FanOut(Vec<Stage>, JoinPolicy),
}

/// Immutable pipeline topology.
pub struct PipelinePlan {
    groups: Vec<StageGroup>,
}

impl PipelinePlan {
    /// Creates a serial pipeline.
    #[must_use]
    pub fn serial(stages: Vec<Stage>) -> Self {
        Self {
            groups: vec![StageGroup::Serial(stages)],
        }
    }

    /// Creates one bounded fan-out and join pipeline.
    #[must_use]
    pub fn fan_out(stages: Vec<Stage>, join: JoinPolicy) -> Self {
        Self {
            groups: vec![StageGroup::FanOut(stages, join)],
        }
    }
}

/// Error category safe for payload-free lifecycle events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageErrorKind {
    /// Transient failure eligible for retry.
    Retryable,
    /// Non-retryable stage failure.
    Fatal,
    /// Pipeline or scope cancellation.
    Cancelled,
    /// Total stage deadline elapsed.
    Deadline,
    /// Circuit was already open.
    CircuitOpen,
    /// Child task panicked or was aborted.
    Join,
}

/// Normalized stage execution failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StageError {
    /// Transient stage failure.
    #[error("retryable stage failure: {message}")]
    Retryable {
        /// Sanitized explanation. Lifecycle events never include it.
        message: String,
    },
    /// Permanent stage failure.
    #[error("fatal stage failure: {message}")]
    Fatal {
        /// Sanitized explanation. Lifecycle events never include it.
        message: String,
    },
    /// Explicit cancellation.
    #[error("pipeline cancelled")]
    Cancelled,
    /// Stage deadline elapsed.
    #[error("stage '{stage}' exceeded its deadline")]
    Deadline {
        /// Stage name.
        stage: String,
    },
    /// Circuit denied execution.
    #[error("circuit for stage '{stage}' is open")]
    CircuitOpen {
        /// Stage name.
        stage: String,
    },
    /// Structured child task did not join normally.
    #[error("pipeline child task failed to join")]
    Join,
}

impl StageError {
    /// Creates a retryable error.
    #[must_use]
    pub fn retryable(message: impl Into<String>) -> Self {
        Self::Retryable {
            message: message.into(),
        }
    }

    /// Returns the payload-free policy category.
    #[must_use]
    pub fn kind(&self) -> StageErrorKind {
        match self {
            Self::Retryable { .. } => StageErrorKind::Retryable,
            Self::Fatal { .. } => StageErrorKind::Fatal,
            Self::Cancelled => StageErrorKind::Cancelled,
            Self::Deadline { .. } => StageErrorKind::Deadline,
            Self::CircuitOpen { .. } => StageErrorKind::CircuitOpen,
            Self::Join => StageErrorKind::Join,
        }
    }
}

/// Payload-free pipeline lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineEvent {
    /// Root scope started.
    Started,
    /// One stage attempt started.
    StageAttempt {
        /// Stage name.
        stage: String,
        /// One-based attempt number.
        attempt: u32,
    },
    /// Stage completed.
    StageSucceeded {
        /// Stage name.
        stage: String,
    },
    /// Stage failed terminally.
    StageFailed {
        /// Stage name.
        stage: String,
        /// Sanitized policy category.
        kind: StageErrorKind,
    },
    /// Fan-out scope opened.
    FanOutStarted {
        /// Number of declared branches.
        width: usize,
    },
    /// Fan-out children joined.
    Joined {
        /// Number of joined children.
        count: usize,
    },
    /// Root scope observed cancellation.
    Cancelled,
    /// Pipeline completed normally.
    Completed,
}

/// Successful pipeline result and ordered metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineOutcome {
    /// Final branch outputs.
    pub output: Vec<String>,
    /// Ordered payload-free lifecycle metadata.
    pub events: Vec<PipelineEvent>,
}

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Clone, Debug, Default)]
struct PipelineCancellation(Arc<CancellationState>);

impl PipelineCancellation {
    fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            self.0.notify.notify_waiters();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.0.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

/// Context inherited by every structured child stage.
#[derive(Clone, Debug)]
pub struct StageContext {
    cancellation: PipelineCancellation,
}

impl StageContext {
    /// Completes when the pipeline or its parent scope is cancelled.
    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }

    /// Reports whether cancellation has already been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

#[derive(Debug, Clone)]
struct CircuitState {
    failures: u32,
    opened_at: Option<Instant>,
}

/// Invalid production pipeline executor configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProductionPipelineConfigError {
    /// Lifecycle buffer cannot be empty.
    #[error("pipeline event capacity must be greater than zero")]
    ZeroEventCapacity,
    /// No branch could execute.
    #[error("pipeline maximum fan-out must be greater than zero")]
    ZeroFanOut,
}

/// Shared production executor with circuit state across invocations.
#[derive(Clone)]
pub struct ProductionPipelineExecutor {
    event_capacity: usize,
    max_fan_out: usize,
    circuits: Arc<Mutex<BTreeMap<String, CircuitState>>>,
}

impl ProductionPipelineExecutor {
    /// Creates a bounded executor.
    ///
    /// # Errors
    ///
    /// Rejects zero event capacity or zero fan-out.
    pub fn new(
        event_capacity: usize,
        max_fan_out: usize,
    ) -> Result<Self, ProductionPipelineConfigError> {
        if event_capacity == 0 {
            return Err(ProductionPipelineConfigError::ZeroEventCapacity);
        }
        if max_fan_out == 0 {
            return Err(ProductionPipelineConfigError::ZeroFanOut);
        }
        Ok(Self {
            event_capacity,
            max_fan_out,
            circuits: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Starts one root pipeline scope.
    #[must_use]
    pub fn execute(&self, plan: PipelinePlan, input: String) -> PipelineHandle {
        let cancellation = PipelineCancellation::default();
        let executor = self.clone();
        let child_cancellation = cancellation.clone();
        let task = tokio::spawn(async move { executor.run(plan, input, child_cancellation).await });
        PipelineHandle { cancellation, task }
    }

    async fn run(
        &self,
        plan: PipelinePlan,
        input: String,
        cancellation: PipelineCancellation,
    ) -> Result<PipelineOutcome, StageError> {
        let mut events = Vec::with_capacity(self.event_capacity);
        events.push(PipelineEvent::Started);
        let mut outputs = vec![input];
        for group in plan.groups {
            if cancellation.is_cancelled() {
                events.push(PipelineEvent::Cancelled);
                return Err(StageError::Cancelled);
            }
            match group {
                StageGroup::Serial(stages) => {
                    let mut output = outputs.pop().ok_or_else(|| StageError::Fatal {
                        message: "serial stage has no input".into(),
                    })?;
                    for stage in stages {
                        output = self
                            .run_stage(stage, output, cancellation.clone(), &mut events)
                            .await?;
                    }
                    outputs = vec![output];
                }
                StageGroup::FanOut(stages, join) => {
                    let input = outputs.pop().ok_or_else(|| StageError::Fatal {
                        message: "fan-out has no input".into(),
                    })?;
                    events.push(PipelineEvent::FanOutStarted {
                        width: stages.len(),
                    });
                    outputs = self
                        .run_fan_out(stages, join, input, cancellation.clone(), &mut events)
                        .await?;
                    events.push(PipelineEvent::Joined {
                        count: outputs.len(),
                    });
                }
            }
        }
        events.push(PipelineEvent::Completed);
        Ok(PipelineOutcome {
            output: outputs,
            events,
        })
    }

    async fn run_fan_out(
        &self,
        stages: Vec<Stage>,
        join: JoinPolicy,
        input: String,
        cancellation: PipelineCancellation,
        events: &mut Vec<PipelineEvent>,
    ) -> Result<Vec<String>, StageError> {
        let semaphore = Arc::new(Semaphore::new(self.max_fan_out));
        let mut children = JoinSet::new();
        for (index, stage) in stages.into_iter().enumerate() {
            let semaphore = Arc::clone(&semaphore);
            let executor = self.clone();
            let input = input.clone();
            let cancellation = cancellation.clone();
            children.spawn(async move {
                let permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| StageError::Join)?;
                let mut child_events = Vec::new();
                let result = executor
                    .run_stage(stage, input, cancellation, &mut child_events)
                    .await;
                drop(permit);
                result.map(|output| (index, output, child_events))
            });
        }
        let mut results = Vec::new();
        while let Some(result) = children.join_next().await {
            let (index, output, child_events) = flatten_join(result)?;
            events.extend(child_events);
            results.push((index, output));
        }
        if join == JoinPolicy::PreserveInputOrder {
            results.sort_by_key(|(index, _)| *index);
        }
        Ok(results.into_iter().map(|(_, output)| output).collect())
    }

    async fn run_stage(
        &self,
        stage: Stage,
        input: String,
        cancellation: PipelineCancellation,
        events: &mut Vec<PipelineEvent>,
    ) -> Result<String, StageError> {
        self.check_circuit(&stage).await?;
        let name = stage.name.clone();
        let operation = self.run_stage_attempts(&stage, input, cancellation, events);
        let result = match stage.deadline {
            Some(deadline) => {
                timeout(deadline, operation)
                    .await
                    .map_err(|_| StageError::Deadline {
                        stage: name.clone(),
                    })?
            }
            None => operation.await,
        };
        match &result {
            Ok(_) => {
                self.record_circuit_success(&stage).await;
                events.push(PipelineEvent::StageSucceeded { stage: name });
            }
            Err(error) => {
                self.record_circuit_failure(&stage).await;
                events.push(PipelineEvent::StageFailed {
                    stage: name,
                    kind: error.kind(),
                });
            }
        }
        result
    }

    async fn run_stage_attempts(
        &self,
        stage: &Stage,
        input: String,
        cancellation: PipelineCancellation,
        events: &mut Vec<PipelineEvent>,
    ) -> Result<String, StageError> {
        for attempt in 1..=stage.retry.max_attempts {
            if cancellation.is_cancelled() {
                return Err(StageError::Cancelled);
            }
            events.push(PipelineEvent::StageAttempt {
                stage: stage.name.clone(),
                attempt,
            });
            let context = StageContext {
                cancellation: cancellation.clone(),
            };
            let result = stage.operation.call(context, input.clone()).await;
            match result {
                Ok(output) => return Ok(output),
                Err(StageError::Retryable { .. }) if attempt < stage.retry.max_attempts => {
                    tokio::select! {
                        biased;
                        () = cancellation.cancelled() => return Err(StageError::Cancelled),
                        () = tokio::time::sleep(stage.retry.backoff) => {}
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(StageError::Fatal {
            message: "retry policy exhausted without terminal result".into(),
        })
    }

    async fn check_circuit(&self, stage: &Stage) -> Result<(), StageError> {
        let Some(policy) = stage.circuit else {
            return Ok(());
        };
        let mut circuits = self.circuits.lock().await;
        let circuit_state = circuits.entry(stage.name.clone()).or_insert(CircuitState {
            failures: 0,
            opened_at: None,
        });
        if let Some(opened_at) = circuit_state.opened_at {
            if opened_at.elapsed() < policy.reset_after {
                return Err(StageError::CircuitOpen {
                    stage: stage.name.clone(),
                });
            }
            circuit_state.failures = 0;
            circuit_state.opened_at = None;
        }
        Ok(())
    }

    async fn record_circuit_success(&self, stage: &Stage) {
        if stage.circuit.is_none() {
            return;
        }
        if let Some(circuit_state) = self.circuits.lock().await.get_mut(&stage.name) {
            circuit_state.failures = 0;
            circuit_state.opened_at = None;
        }
    }

    async fn record_circuit_failure(&self, stage: &Stage) {
        let Some(policy) = stage.circuit else {
            return;
        };
        let mut circuits = self.circuits.lock().await;
        let circuit_state = circuits.entry(stage.name.clone()).or_insert(CircuitState {
            failures: 0,
            opened_at: None,
        });
        circuit_state.failures = circuit_state.failures.saturating_add(1);
        if circuit_state.failures >= policy.failure_threshold {
            circuit_state.opened_at = Some(Instant::now());
        }
    }
}

fn flatten_join(result: JoinedBranch) -> Result<BranchOutput, StageError> {
    result.map_err(|_| StageError::Join)?
}

/// Running structured pipeline scope.
pub struct PipelineHandle {
    cancellation: PipelineCancellation,
    task: tokio::task::JoinHandle<Result<PipelineOutcome, StageError>>,
}

impl PipelineHandle {
    /// Requests idempotent cancellation of the root and all children.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Waits until every child joins and returns the terminal result.
    ///
    /// # Errors
    ///
    /// Returns stage policy failures, cancellation, deadline, circuit, or child
    /// join failure.
    pub async fn wait(self) -> Result<PipelineOutcome, StageError> {
        self.task.await.map_err(|_| StageError::Join)?
    }
}
