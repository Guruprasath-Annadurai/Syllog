//! Verified control-flow intermediate representation for Syllog.

mod verify;

pub use verify::{VerificationError, verify};

use serde::{Deserialize, Serialize};

/// Module component of a stable definition identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DefId {
    /// Package-local module index.
    pub module: u32,
    /// Module-local definition index.
    pub index: u32,
}

/// Basic-block index within a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockId(pub u32);

/// Local storage index within a function. Local zero is the return destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocalId(pub u32);

/// Stable state index within one lowered async function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AsyncStateId(pub u32);

/// Explicit transition in a verified async state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncTransition {
    /// Enters the first executable state.
    Start {
        /// First suspension or completion state.
        next: AsyncStateId,
    },
    /// Yields execution until the awaited operation wakes the task.
    Suspend {
        /// Zero-based source-order await point.
        await_index: u32,
        /// State entered after a wake.
        resume: AsyncStateId,
        /// Shared cancellation/drop state.
        cancel: AsyncStateId,
    },
    /// Restores live locals and continues execution.
    Resume {
        /// Next suspension or completion state.
        next: AsyncStateId,
        /// Shared panic/drop state.
        panic: AsyncStateId,
    },
    /// Returns the function result.
    Complete,
    /// Runs the single terminal drop path after cancellation or panic.
    Cancel {
        /// Locals whose deterministic drop flags are stored in the frame.
        drop_locals: Vec<DefId>,
    },
}

/// One tagged async-machine state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncState {
    /// Contiguous state identity.
    pub id: AsyncStateId,
    /// Transition executed from this state.
    pub transition: AsyncTransition,
}

/// Verified async frame and transition graph for one source function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncStateMachine {
    /// Source function identity.
    pub function: DefId,
    /// Every task must be attached to a structured parent scope.
    pub parent_scope_required: bool,
    /// Locals conservatively retained across suspension.
    pub live_locals: Vec<DefId>,
    /// Contiguous state table beginning at state zero.
    pub states: Vec<AsyncState>,
}

/// Invalid async-machine graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncVerificationError {
    /// The state table is empty or IDs are not contiguous.
    InvalidStateTable,
    /// A transition targets a missing state.
    InvalidTarget {
        /// Referencing state.
        state: AsyncStateId,
        /// Missing target.
        target: AsyncStateId,
    },
    /// There is not exactly one completion and one cancellation/drop state.
    InvalidTerminalCount,
    /// An async task has no mandatory parent scope.
    MissingParentScope,
}

/// Verifies structural async state-machine invariants.
///
/// # Errors
///
/// Returns deterministic graph errors for invalid IDs, targets, terminal paths,
/// or missing structured-parent requirements.
pub fn verify_async_machine(
    machine: &AsyncStateMachine,
) -> Result<(), Vec<AsyncVerificationError>> {
    let mut errors = Vec::new();
    if machine.states.is_empty()
        || machine
            .states
            .iter()
            .enumerate()
            .any(|(index, state)| state.id.0 != u32::try_from(index).unwrap_or(u32::MAX))
    {
        errors.push(AsyncVerificationError::InvalidStateTable);
    }
    if !machine.parent_scope_required {
        errors.push(AsyncVerificationError::MissingParentScope);
    }
    let length = machine.states.len();
    for state in &machine.states {
        let targets = match state.transition {
            AsyncTransition::Start { next } => vec![next],
            AsyncTransition::Suspend { resume, cancel, .. } => vec![resume, cancel],
            AsyncTransition::Resume { next, panic } => vec![next, panic],
            AsyncTransition::Complete | AsyncTransition::Cancel { .. } => Vec::new(),
        };
        for target in targets {
            if usize::try_from(target.0).map_or(true, |index| index >= length) {
                errors.push(AsyncVerificationError::InvalidTarget {
                    state: state.id,
                    target,
                });
            }
        }
    }
    let complete = machine
        .states
        .iter()
        .filter(|state| matches!(state.transition, AsyncTransition::Complete))
        .count();
    let cancel = machine
        .states
        .iter()
        .filter(|state| matches!(state.transition, AsyncTransition::Cancel { .. }))
        .count();
    if complete != 1 || cancel != 1 {
        errors.push(AsyncVerificationError::InvalidTerminalCount);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Runtime-representable MIR types in the first executable subset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirType {
    /// No value.
    Unit,
    /// Boolean value.
    Bool,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 64-bit integer.
    U64,
    /// Owned UTF-8 string.
    String,
    /// User aggregate identified by its definition.
    Aggregate(DefId),
}

/// Literal MIR constant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Constant {
    /// Unit.
    Unit,
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    I64(i64),
    /// Unsigned integer.
    U64(u64),
    /// UTF-8 text.
    String(String),
}

impl Constant {
    /// Returns this constant's MIR type.
    #[must_use]
    pub fn ty(&self) -> MirType {
        match self {
            Self::Unit => MirType::Unit,
            Self::Bool(_) => MirType::Bool,
            Self::I64(_) => MirType::I64,
            Self::U64(_) => MirType::U64,
            Self::String(_) => MirType::String,
        }
    }
}

/// A value consumed by an rvalue or terminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operand {
    /// Inline constant.
    Constant(Constant),
    /// Non-consuming local read.
    Copy(LocalId),
    /// Consuming local read.
    Move(LocalId),
}

/// An assignable or droppable storage location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    /// Complete local.
    Local(LocalId),
    /// Positional aggregate field.
    Field {
        /// Aggregate local.
        base: LocalId,
        /// Zero-based field index.
        field: u32,
    },
}

/// Primitive MIR binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    /// Addition.
    Add,
    /// Subtraction.
    Subtract,
    /// Multiplication.
    Multiply,
    /// Integer division.
    Divide,
    /// Equality comparison.
    Equal,
    /// Less-than comparison.
    Less,
    /// Boolean conjunction.
    And,
    /// Boolean disjunction.
    Or,
}

/// Computation assigned to a place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rvalue {
    /// Direct operand use.
    Use(Operand),
    /// Primitive binary operation.
    Binary {
        /// Operation.
        operator: BinaryOp,
        /// Left input.
        left: Operand,
        /// Right input.
        right: Operand,
    },
    /// Aggregate construction.
    Aggregate {
        /// Aggregate result type.
        ty: DefId,
        /// Stable enum tag, or zero for product types.
        discriminant: u64,
        /// Ordered field values.
        fields: Vec<Operand>,
    },
    /// Reads the stable numeric tag of an aggregate value.
    Discriminant(Operand),
}

/// Side-effecting instruction within a basic block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Statement {
    /// Computes and stores a value.
    Assign {
        /// Destination place.
        destination: Place,
        /// Computation.
        value: Rvalue,
    },
    /// Runs deterministic destruction for a place.
    Drop(Place),
}

/// Mandatory control-flow instruction ending a basic block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Terminator {
    /// Returns local zero to the caller.
    Return,
    /// Unconditional branch.
    Goto(BlockId),
    /// Integer or Boolean dispatch.
    SwitchInt {
        /// Scrutinee.
        value: Operand,
        /// Exact-value edges.
        targets: Vec<(u128, BlockId)>,
        /// Default edge.
        otherwise: BlockId,
    },
    /// Direct function call and continuation edge.
    Call {
        /// Callee identity.
        function: DefId,
        /// Positional arguments.
        args: Vec<Operand>,
        /// Result destination.
        destination: Place,
        /// Successful continuation.
        next: BlockId,
    },
}

/// Straight-line statements followed by exactly one terminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicBlock {
    /// Ordered statements.
    pub statements: Vec<Statement>,
    /// Optional only while constructing MIR; verified MIR always has one.
    pub terminator: Option<Terminator>,
}

/// One control-flow function. `locals[0]` is its return destination and
/// parameters occupy the following `parameter_count` locals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirFunction {
    /// Definition identity.
    pub id: DefId,
    /// Number of parameter locals after local zero.
    pub parameter_count: u32,
    /// Declared result type.
    pub return_type: MirType,
    /// Local types by local identity.
    pub locals: Vec<MirType>,
    /// Control-flow blocks; block zero is entry.
    pub blocks: Vec<BasicBlock>,
}

/// Complete MIR package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirProgram {
    /// Functions in stable definition order.
    pub functions: Vec<MirFunction>,
    /// Executable package entry when one is declared.
    pub entry: Option<DefId>,
}
