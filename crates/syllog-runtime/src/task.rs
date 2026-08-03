//! Execution of verified async state machines with explicit parent scopes.

use syllog_ir::{
    AsyncStateId, AsyncStateMachine, AsyncTransition, AsyncVerificationError, DefId,
    verify_async_machine,
};

/// Stable structured-concurrency parent identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentScopeId(pub u64);

/// Deterministic scheduler intervention used by tests and replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskControl {
    /// Wake every suspension normally.
    Complete,
    /// Cancel at one source-order await point.
    CancelAt {
        /// Await point that observes cancellation.
        await_index: u32,
    },
    /// Inject a panic after resuming one await point.
    PanicAt {
        /// Await point after which panic propagation begins.
        await_index: u32,
    },
}

/// Observable task lifecycle metadata. It intentionally contains no payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEvent {
    /// Task entered its structured parent scope.
    Started {
        /// Owning scope.
        parent: ParentScopeId,
    },
    /// Task yielded at an await point.
    Suspended {
        /// Source-order await point.
        await_index: u32,
    },
    /// Wake handle scheduled the continuation.
    Woken {
        /// Source-order await point.
        await_index: u32,
    },
    /// Live frame state was restored.
    Resumed {
        /// Source-order await point.
        await_index: u32,
    },
    /// Cancellation was observed.
    Cancelled,
    /// Panic propagation started.
    Panicked,
    /// Shared drop path ran once.
    Dropped {
        /// Frame locals covered by deterministic drop flags.
        locals: Vec<DefId>,
    },
    /// Task returned successfully.
    Completed,
}

/// Async task execution failure with ordered lifecycle evidence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaskFailure {
    /// Input state machine was not structurally valid.
    #[error("invalid async state machine: {errors:?}")]
    InvalidMachine {
        /// Verifier failures.
        errors: Vec<AsyncVerificationError>,
    },
    /// A valid graph reached an impossible runtime state.
    #[error("async state machine reached invalid runtime state {state:?}")]
    InvalidRuntimeState {
        /// Missing or inconsistent state.
        state: AsyncStateId,
    },
    /// Task panicked after its drop path completed.
    #[error("async task panicked")]
    Panicked {
        /// Ordered, payload-free lifecycle events.
        events: Vec<TaskEvent>,
    },
}

enum Step {
    Pending {
        await_index: u32,
        resume: AsyncStateId,
    },
    Complete,
}

struct Runner<'a> {
    machine: &'a AsyncStateMachine,
    state: AsyncStateId,
    control: TaskControl,
    last_await: Option<u32>,
    events: Vec<TaskEvent>,
    panicked: bool,
}

impl<'a> Runner<'a> {
    fn new(
        machine: &'a AsyncStateMachine,
        parent: ParentScopeId,
        control: TaskControl,
    ) -> Result<Self, TaskFailure> {
        verify_async_machine(machine).map_err(|errors| TaskFailure::InvalidMachine { errors })?;
        Ok(Self {
            machine,
            state: AsyncStateId(0),
            control,
            last_await: None,
            events: vec![TaskEvent::Started { parent }],
            panicked: false,
        })
    }

    fn advance(&mut self) -> Result<Step, TaskFailure> {
        loop {
            let state = self
                .machine
                .states
                .get(usize::try_from(self.state.0).unwrap_or(usize::MAX))
                .ok_or(TaskFailure::InvalidRuntimeState { state: self.state })?;
            match &state.transition {
                AsyncTransition::Start { next } => self.state = *next,
                AsyncTransition::Suspend {
                    await_index,
                    resume,
                    cancel,
                } => {
                    self.events.push(TaskEvent::Suspended {
                        await_index: *await_index,
                    });
                    if self.control
                        == (TaskControl::CancelAt {
                            await_index: *await_index,
                        })
                    {
                        self.events.push(TaskEvent::Cancelled);
                        self.state = *cancel;
                    } else {
                        return Ok(Step::Pending {
                            await_index: *await_index,
                            resume: *resume,
                        });
                    }
                }
                AsyncTransition::Resume { next, panic } => {
                    let await_index = self
                        .last_await
                        .ok_or(TaskFailure::InvalidRuntimeState { state: self.state })?;
                    self.events.push(TaskEvent::Resumed { await_index });
                    if self.control == (TaskControl::PanicAt { await_index }) {
                        self.events.push(TaskEvent::Panicked);
                        self.panicked = true;
                        self.state = *panic;
                    } else {
                        self.state = *next;
                    }
                }
                AsyncTransition::Complete => {
                    self.events.push(TaskEvent::Completed);
                    return Ok(Step::Complete);
                }
                AsyncTransition::Cancel { drop_locals } => {
                    self.events.push(TaskEvent::Dropped {
                        locals: drop_locals.clone(),
                    });
                    return if self.panicked {
                        Err(TaskFailure::Panicked {
                            events: std::mem::take(&mut self.events),
                        })
                    } else {
                        Ok(Step::Complete)
                    };
                }
            }
        }
    }

    fn wake(&mut self, await_index: u32, resume: AsyncStateId) {
        self.events.push(TaskEvent::Woken { await_index });
        self.last_await = Some(await_index);
        self.state = resume;
    }
}

/// Runs a verified task with an immediate deterministic wake scheduler.
///
/// # Errors
///
/// Returns verifier, runtime-state, or propagated panic failures.
pub fn run_deterministic_task(
    machine: &AsyncStateMachine,
    parent: ParentScopeId,
    control: TaskControl,
) -> Result<Vec<TaskEvent>, TaskFailure> {
    let mut runner = Runner::new(machine, parent, control)?;
    loop {
        match runner.advance()? {
            Step::Pending {
                await_index,
                resume,
            } => runner.wake(await_index, resume),
            Step::Complete => return Ok(runner.events),
        }
    }
}

/// Runs a verified task through Tokio wake yields.
///
/// # Errors
///
/// Returns verifier, runtime-state, or propagated panic failures.
pub async fn run_tokio_task(
    machine: AsyncStateMachine,
    parent: ParentScopeId,
    control: TaskControl,
) -> Result<Vec<TaskEvent>, TaskFailure> {
    let mut runner = Runner::new(&machine, parent, control)?;
    loop {
        match runner.advance()? {
            Step::Pending {
                await_index,
                resume,
            } => {
                tokio::task::yield_now().await;
                runner.wake(await_index, resume);
            }
            Step::Complete => return Ok(runner.events),
        }
    }
}
