//! Lowering of typed async HIR into explicit verified state machines.

use syllog_ir::{
    AsyncState, AsyncStateId, AsyncStateMachine, AsyncTransition, AsyncVerificationError,
    verify_async_machine,
};

use crate::hir::{DefId, HirDefinitionKind, HirExprKind, HirProgram, HirStatement, TypedExpr};
use syllog_semantic::{PrimitiveType, ResolvedType};

/// Async lowering failed verification.
#[derive(Debug, thiserror::Error)]
pub enum AsyncLowerError {
    /// Generated state-machine graph violated a backend invariant.
    #[error("generated async state machine failed verification: {0:?}")]
    Verification(Vec<AsyncVerificationError>),
    /// A borrowed value would be retained in a resumable frame.
    #[error("async function {function:?} retains borrowed locals across await: {locals:?}")]
    BorrowAcrossAwait {
        /// Affected async function.
        function: syllog_ir::DefId,
        /// Borrowed parameters or locals conservatively live at suspension.
        locals: Vec<syllog_ir::DefId>,
    },
}

/// Lowers every async HIR function into a tagged, verified transition graph.
///
/// # Errors
///
/// Returns an error if any generated graph violates async MIR invariants.
pub fn lower_async_state_machines(
    program: &HirProgram,
) -> Result<Vec<AsyncStateMachine>, AsyncLowerError> {
    let mut machines = Vec::new();
    for definition in program
        .modules
        .iter()
        .flat_map(|module| &module.definitions)
    {
        let HirDefinitionKind::Function(function) = &definition.kind else {
            continue;
        };
        if !function.asynchronous {
            continue;
        }
        let mut awaits = 0_u32;
        let mut locals = function
            .parameters
            .iter()
            .map(|parameter| map_id(parameter.id))
            .collect::<Vec<_>>();
        for statement in &function.body.statements {
            if let HirStatement::Let { definition, .. } = statement {
                locals.push(map_id(*definition));
            }
            count_statement_awaits(statement, &mut awaits);
        }
        let borrowed = function
            .parameters
            .iter()
            .filter(|parameter| contains_borrow(&parameter.ty))
            .map(|parameter| map_id(parameter.id))
            .chain(
                function
                    .body
                    .statements
                    .iter()
                    .filter_map(|statement| match statement {
                        HirStatement::Let { definition, ty, .. } if contains_borrow(ty) => {
                            Some(map_id(*definition))
                        }
                        _ => None,
                    }),
            )
            .collect::<Vec<_>>();
        if awaits > 0 && !borrowed.is_empty() {
            return Err(AsyncLowerError::BorrowAcrossAwait {
                function: map_id(definition.id),
                locals: borrowed,
            });
        }
        locals.sort();
        locals.dedup();
        let machine = build_machine(map_id(definition.id), awaits, locals);
        verify_async_machine(&machine).map_err(AsyncLowerError::Verification)?;
        machines.push(machine);
    }
    Ok(machines)
}

fn contains_borrow(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Primitive(PrimitiveType::Str) => true,
        ResolvedType::Option(inner) | ResolvedType::Array(inner) => contains_borrow(inner),
        ResolvedType::Result(ok, error) => contains_borrow(ok) || contains_borrow(error),
        ResolvedType::Tuple(items) => items.iter().any(contains_borrow),
        ResolvedType::Function(parameters, result) => {
            parameters.iter().any(contains_borrow) || contains_borrow(result)
        }
        _ => false,
    }
}

fn build_machine(
    function: syllog_ir::DefId,
    awaits: u32,
    live_locals: Vec<syllog_ir::DefId>,
) -> AsyncStateMachine {
    let complete_id = AsyncStateId(1 + awaits.saturating_mul(2));
    let cancel_id = AsyncStateId(complete_id.0 + 1);
    let mut states = vec![AsyncState {
        id: AsyncStateId(0),
        transition: AsyncTransition::Start {
            next: if awaits == 0 {
                complete_id
            } else {
                AsyncStateId(1)
            },
        },
    }];
    for await_index in 0..awaits {
        let suspend = AsyncStateId(1 + await_index * 2);
        let resume = AsyncStateId(suspend.0 + 1);
        let next = if await_index + 1 == awaits {
            complete_id
        } else {
            AsyncStateId(suspend.0 + 2)
        };
        states.push(AsyncState {
            id: suspend,
            transition: AsyncTransition::Suspend {
                await_index,
                resume,
                cancel: cancel_id,
            },
        });
        states.push(AsyncState {
            id: resume,
            transition: AsyncTransition::Resume {
                next,
                panic: cancel_id,
            },
        });
    }
    states.push(AsyncState {
        id: complete_id,
        transition: AsyncTransition::Complete,
    });
    states.push(AsyncState {
        id: cancel_id,
        transition: AsyncTransition::Cancel {
            drop_locals: live_locals.clone(),
        },
    });
    AsyncStateMachine {
        function,
        parent_scope_required: true,
        live_locals,
        states,
    }
}

fn count_statement_awaits(statement: &HirStatement, count: &mut u32) {
    match statement {
        HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
            count_expression_awaits(value, count);
        }
        HirStatement::Return(value) => {
            if let Some(value) = value {
                count_expression_awaits(value, count);
            }
        }
    }
}

fn count_expression_awaits(expression: &TypedExpr, count: &mut u32) {
    expression.walk(&mut |expression| {
        if matches!(expression.kind, HirExprKind::Await(_)) {
            *count = count.saturating_add(1);
        }
    });
}

fn map_id(id: DefId) -> syllog_ir::DefId {
    syllog_ir::DefId {
        module: id.module.0,
        index: id.index,
    }
}
