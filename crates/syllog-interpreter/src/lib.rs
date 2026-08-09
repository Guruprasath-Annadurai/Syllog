//! Deterministic reference execution for verified Syllog MIR.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use syllog_ir::{
    BinaryOp, Constant, DefId, LocalId, MirFunction, MirProgram, Operand, Place, Rvalue, Statement,
    Terminator, verify,
};
use thiserror::Error;

/// Public runtime value produced by reference execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeValue {
    /// Statically validated reference value in the instrumented interpreter.
    Reference(Box<RuntimeValue>),
    /// Unit.
    Unit,
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    I64(i64),
    /// Unsigned 64-bit integer.
    U64(u64),
    /// Owned UTF-8 text.
    String(String),
    /// Aggregate fields, with representation details intentionally hidden.
    Aggregate(Vec<RuntimeValue>),
}

/// Observable result of reference execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Entry function return value.
    pub value: RuntimeValue,
    /// Bytes written through the standard output capability.
    pub stdout: Vec<u8>,
    /// Number of successful explicit MIR drops, for conformance instrumentation.
    pub drops_executed: u64,
}

/// Deterministic hard limits applied to one execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpreterLimits {
    /// Maximum executed statements and terminators.
    pub max_instructions: u64,
    /// Maximum active function frames.
    pub max_call_depth: u32,
    /// Maximum bytes charged for strings and aggregate storage.
    pub max_allocated_bytes: u64,
    /// Maximum standard output bytes.
    pub max_output_bytes: u64,
}

impl Default for InterpreterLimits {
    fn default() -> Self {
        Self {
            max_instructions: 1_000_000,
            max_call_depth: 256,
            max_allocated_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
        }
    }
}

/// Deterministic reference-interpreter failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeError {
    /// MIR failed verification before execution.
    #[error("MIR verification failed: {0}")]
    InvalidMir(String),
    /// Requested entry function is absent.
    #[error("entry function {entry:?} does not exist")]
    MissingEntry {
        /// Missing definition.
        entry: DefId,
    },
    /// Instruction budget was exhausted.
    #[error("instruction limit {limit} exceeded")]
    InstructionLimitExceeded {
        /// Configured limit.
        limit: u64,
    },
    /// Call-depth budget was exhausted.
    #[error("call-depth limit {limit} exceeded")]
    CallDepthLimitExceeded {
        /// Configured limit.
        limit: u32,
    },
    /// Allocation budget was exhausted.
    #[error("allocation limit {limit} bytes exceeded")]
    AllocationLimitExceeded {
        /// Configured limit.
        limit: u64,
    },
    /// Output budget was exhausted.
    #[error("output limit {limit} bytes exceeded")]
    OutputLimitExceeded {
        /// Configured limit.
        limit: u64,
    },
    /// A dynamically checked operation failed.
    #[error("runtime trap: {message}")]
    Trap {
        /// Stable human-readable trap reason.
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Reference(Box<Value>),
    Unit,
    Bool(bool),
    I64(i64),
    U64(u64),
    String(String),
    Aggregate {
        discriminant: u64,
        fields: Vec<Value>,
    },
}

impl Value {
    fn into_runtime(self) -> RuntimeValue {
        match self {
            Self::Reference(value) => RuntimeValue::Reference(Box::new(value.into_runtime())),
            Self::Unit => RuntimeValue::Unit,
            Self::Bool(value) => RuntimeValue::Bool(value),
            Self::I64(value) => RuntimeValue::I64(value),
            Self::U64(value) => RuntimeValue::U64(value),
            Self::String(value) => RuntimeValue::String(value),
            Self::Aggregate { fields, .. } => {
                RuntimeValue::Aggregate(fields.into_iter().map(Self::into_runtime).collect())
            }
        }
    }
}

/// Executes verified MIR with deterministic resource accounting.
///
/// # Errors
///
/// Returns verification, lookup, resource-limit, or checked-operation errors.
pub fn execute(
    program: &MirProgram,
    entry: DefId,
    limits: InterpreterLimits,
) -> Result<ExecutionResult, RuntimeError> {
    verify(program).map_err(|errors| RuntimeError::InvalidMir(format!("{errors:?}")))?;
    let functions = program
        .functions
        .iter()
        .map(|function| (function.id, function))
        .collect();
    let mut machine = Machine {
        functions,
        limits,
        instructions: 0,
        allocated_bytes: 0,
        stdout: Vec::new(),
        drops_executed: 0,
    };
    let value = machine.call(entry, Vec::new(), 1)?;
    Ok(ExecutionResult {
        value: value.into_runtime(),
        stdout: machine.stdout,
        drops_executed: machine.drops_executed,
    })
}

struct Machine<'a> {
    functions: BTreeMap<DefId, &'a MirFunction>,
    limits: InterpreterLimits,
    instructions: u64,
    allocated_bytes: u64,
    stdout: Vec<u8>,
    drops_executed: u64,
}

impl Machine<'_> {
    fn call(
        &mut self,
        id: DefId,
        arguments: Vec<Value>,
        depth: u32,
    ) -> Result<Value, RuntimeError> {
        if depth > self.limits.max_call_depth {
            return Err(RuntimeError::CallDepthLimitExceeded {
                limit: self.limits.max_call_depth,
            });
        }
        let function = self
            .functions
            .get(&id)
            .copied()
            .cloned()
            .ok_or(RuntimeError::MissingEntry { entry: id })?;
        if arguments.len() != usize::try_from(function.parameter_count).unwrap_or(usize::MAX) {
            return trap_result("verified call has an invalid argument count");
        }
        let mut locals = vec![None; function.locals.len()];
        for (index, argument) in arguments.into_iter().enumerate() {
            locals[index + 1] = Some(argument);
        }
        let mut block = 0_usize;
        loop {
            let current = function
                .blocks
                .get(block)
                .ok_or_else(|| RuntimeError::Trap {
                    message: "verified block target is absent".into(),
                })?;
            for statement in &current.statements {
                self.tick()?;
                self.execute_statement(statement, &mut locals)?;
            }
            self.tick()?;
            match current
                .terminator
                .as_ref()
                .expect("verified MIR has a terminator")
            {
                Terminator::Return => {
                    return locals[0].take().ok_or_else(|| RuntimeError::Trap {
                        message: "return destination is empty".into(),
                    });
                }
                Terminator::Goto(target) => {
                    block = usize::try_from(target.0).map_err(|_| RuntimeError::Trap {
                        message: "block target does not fit the host".into(),
                    })?;
                }
                Terminator::SwitchInt {
                    value,
                    targets,
                    otherwise,
                } => {
                    let value = self.integer_operand(value, &mut locals)?;
                    let target = targets
                        .iter()
                        .find_map(|(candidate, target)| (*candidate == value).then_some(*target))
                        .unwrap_or(*otherwise);
                    block = usize::try_from(target.0).map_err(|_| RuntimeError::Trap {
                        message: "switch target does not fit the host".into(),
                    })?;
                }
                Terminator::Call {
                    function,
                    args,
                    destination,
                    next,
                } => {
                    let args = args
                        .iter()
                        .map(|argument| self.operand(argument, &mut locals))
                        .collect::<Result<Vec<_>, _>>()?;
                    let value = self.call(*function, args, depth.saturating_add(1))?;
                    Self::assign(destination, value, &mut locals)?;
                    block = usize::try_from(next.0).map_err(|_| RuntimeError::Trap {
                        message: "call continuation does not fit the host".into(),
                    })?;
                }
            }
        }
    }

    fn execute_statement(
        &mut self,
        statement: &Statement,
        locals: &mut [Option<Value>],
    ) -> Result<(), RuntimeError> {
        match statement {
            Statement::Assign { destination, value } => {
                let value = self.rvalue(value, locals)?;
                Self::assign(destination, value, locals)
            }
            Statement::Drop(place) => self.drop_place(place, locals),
        }
    }

    fn rvalue(
        &mut self,
        rvalue: &Rvalue,
        locals: &mut [Option<Value>],
    ) -> Result<Value, RuntimeError> {
        match rvalue {
            Rvalue::Borrow { place, .. } => Self::read_place(place, locals)
                .cloned()
                .map(Box::new)
                .map(Value::Reference),
            Rvalue::Use(operand) => self.operand(operand, locals),
            Rvalue::Discriminant(operand) => match self.operand(operand, locals)? {
                Value::Aggregate { discriminant, .. } => Ok(Value::U64(discriminant)),
                _ => trap_result("discriminant read requires an aggregate"),
            },
            Rvalue::Aggregate {
                discriminant,
                fields,
                ..
            } => {
                let fields = fields
                    .iter()
                    .map(|field| self.operand(field, locals))
                    .collect::<Result<Vec<_>, _>>()?;
                let bytes = u64::try_from(fields.len())
                    .unwrap_or(u64::MAX)
                    .saturating_mul(16);
                self.charge_allocation(bytes)?;
                Ok(Value::Aggregate {
                    discriminant: *discriminant,
                    fields,
                })
            }
            Rvalue::Binary {
                operator,
                left,
                right,
            } => {
                let left = self.operand(left, locals)?;
                let right = self.operand(right, locals)?;
                Self::binary(*operator, left, right)
            }
        }
    }

    fn operand(
        &mut self,
        operand: &Operand,
        locals: &mut [Option<Value>],
    ) -> Result<Value, RuntimeError> {
        match operand {
            Operand::Constant(constant) => self.constant(constant),
            Operand::Copy(local) => local_value(locals, *local).cloned(),
            Operand::Move(local) => {
                local_value_mut(locals, *local)?
                    .take()
                    .ok_or_else(|| RuntimeError::Trap {
                        message: "moved local is empty".into(),
                    })
            }
        }
    }

    fn constant(&mut self, constant: &Constant) -> Result<Value, RuntimeError> {
        Ok(match constant {
            Constant::Unit => Value::Unit,
            Constant::Bool(value) => Value::Bool(*value),
            Constant::I64(value) => Value::I64(*value),
            Constant::U64(value) => Value::U64(*value),
            Constant::String(value) => {
                self.charge_allocation(u64::try_from(value.len()).unwrap_or(u64::MAX))?;
                Value::String(value.clone())
            }
        })
    }

    fn binary(operator: BinaryOp, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (operator, left, right) {
            (BinaryOp::Add, Value::U64(left), Value::U64(right)) => left
                .checked_add(right)
                .map(Value::U64)
                .ok_or_else(|| trap("U64 addition overflow")),
            (BinaryOp::Subtract, Value::U64(left), Value::U64(right)) => left
                .checked_sub(right)
                .map(Value::U64)
                .ok_or_else(|| trap("U64 subtraction overflow")),
            (BinaryOp::Multiply, Value::U64(left), Value::U64(right)) => left
                .checked_mul(right)
                .map(Value::U64)
                .ok_or_else(|| trap("U64 multiplication overflow")),
            (BinaryOp::Divide, Value::U64(left), Value::U64(right)) => left
                .checked_div(right)
                .map(Value::U64)
                .ok_or_else(|| trap("U64 division by zero")),
            (BinaryOp::Add, Value::I64(left), Value::I64(right)) => left
                .checked_add(right)
                .map(Value::I64)
                .ok_or_else(|| trap("I64 addition overflow")),
            (BinaryOp::Subtract, Value::I64(left), Value::I64(right)) => left
                .checked_sub(right)
                .map(Value::I64)
                .ok_or_else(|| trap("I64 subtraction overflow")),
            (BinaryOp::Multiply, Value::I64(left), Value::I64(right)) => left
                .checked_mul(right)
                .map(Value::I64)
                .ok_or_else(|| trap("I64 multiplication overflow")),
            (BinaryOp::Divide, Value::I64(left), Value::I64(right)) => left
                .checked_div(right)
                .map(Value::I64)
                .ok_or_else(|| trap("I64 division failure")),
            (BinaryOp::Equal, left, right) => Ok(Value::Bool(left == right)),
            (BinaryOp::Less, Value::U64(left), Value::U64(right)) => Ok(Value::Bool(left < right)),
            (BinaryOp::Less, Value::I64(left), Value::I64(right)) => Ok(Value::Bool(left < right)),
            (BinaryOp::And, Value::Bool(left), Value::Bool(right)) => {
                Ok(Value::Bool(left && right))
            }
            (BinaryOp::Or, Value::Bool(left), Value::Bool(right)) => Ok(Value::Bool(left || right)),
            _ => trap_result("binary operand types are invalid"),
        }
    }

    fn assign(
        place: &Place,
        value: Value,
        locals: &mut [Option<Value>],
    ) -> Result<(), RuntimeError> {
        match place {
            Place::Local(local) => {
                *local_value_mut(locals, *local)? = Some(value);
                Ok(())
            }
            Place::Field { base, field } => {
                let aggregate = local_value_mut(locals, *base)?
                    .as_mut()
                    .ok_or_else(|| trap("field base is empty"))?;
                let Value::Aggregate { fields, .. } = aggregate else {
                    return trap_result("field base is not an aggregate");
                };
                let field = usize::try_from(*field).map_err(|_| trap("field index overflow"))?;
                let destination = fields
                    .get_mut(field)
                    .ok_or_else(|| trap("field index is out of range"))?;
                *destination = value;
                Ok(())
            }
        }
    }

    fn read_place<'a>(
        place: &Place,
        locals: &'a [Option<Value>],
    ) -> Result<&'a Value, RuntimeError> {
        match place {
            Place::Local(local) => local_value(locals, *local),
            Place::Field { base, field } => {
                let aggregate = local_value(locals, *base)?;
                let Value::Aggregate { fields, .. } = aggregate else {
                    return trap_result("field base is not an aggregate");
                };
                fields
                    .get(usize::try_from(*field).map_err(|_| trap("field index overflow"))?)
                    .ok_or_else(|| trap("field index is out of range"))
            }
        }
    }

    fn drop_place(
        &mut self,
        place: &Place,
        locals: &mut [Option<Value>],
    ) -> Result<(), RuntimeError> {
        match place {
            Place::Local(local) => {
                local_value_mut(locals, *local)?
                    .take()
                    .ok_or_else(|| trap("drop target is empty"))?;
                self.drops_executed = self.drops_executed.saturating_add(1);
                Ok(())
            }
            Place::Field { base, field } => {
                let aggregate = local_value_mut(locals, *base)?
                    .as_mut()
                    .ok_or_else(|| trap("field base is empty"))?;
                let Value::Aggregate { fields, .. } = aggregate else {
                    return trap_result("field base is not an aggregate");
                };
                let field = usize::try_from(*field).map_err(|_| trap("field index overflow"))?;
                *fields
                    .get_mut(field)
                    .ok_or_else(|| trap("field index is out of range"))? = Value::Unit;
                self.drops_executed = self.drops_executed.saturating_add(1);
                Ok(())
            }
        }
    }

    fn integer_operand(
        &mut self,
        operand: &Operand,
        locals: &mut [Option<Value>],
    ) -> Result<u128, RuntimeError> {
        match self.operand(operand, locals)? {
            Value::Bool(value) => Ok(u128::from(value)),
            Value::U64(value) => Ok(u128::from(value)),
            Value::I64(value) => u128::try_from(value).map_err(|_| trap("negative switch value")),
            _ => trap_result("switch value is not an integer"),
        }
    }

    fn tick(&mut self) -> Result<(), RuntimeError> {
        self.instructions = self.instructions.saturating_add(1);
        if self.instructions > self.limits.max_instructions {
            Err(RuntimeError::InstructionLimitExceeded {
                limit: self.limits.max_instructions,
            })
        } else {
            Ok(())
        }
    }

    fn charge_allocation(&mut self, bytes: u64) -> Result<(), RuntimeError> {
        self.allocated_bytes = self.allocated_bytes.saturating_add(bytes);
        if self.allocated_bytes > self.limits.max_allocated_bytes {
            Err(RuntimeError::AllocationLimitExceeded {
                limit: self.limits.max_allocated_bytes,
            })
        } else {
            Ok(())
        }
    }
}

fn local_value(locals: &[Option<Value>], local: LocalId) -> Result<&Value, RuntimeError> {
    locals
        .get(usize::try_from(local.0).map_err(|_| trap("local index overflow"))?)
        .and_then(Option::as_ref)
        .ok_or_else(|| trap("local is empty"))
}

fn local_value_mut(
    locals: &mut [Option<Value>],
    local: LocalId,
) -> Result<&mut Option<Value>, RuntimeError> {
    locals
        .get_mut(usize::try_from(local.0).map_err(|_| trap("local index overflow"))?)
        .ok_or_else(|| trap("local index is out of range"))
}

fn trap(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Trap {
        message: message.into(),
    }
}

fn trap_result<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(trap(message))
}
