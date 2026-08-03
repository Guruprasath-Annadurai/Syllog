//! Structural, type, and definite-initialization verification for MIR.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{
    BasicBlock, BinaryOp, BlockId, DefId, LocalId, MirFunction, MirProgram, MirType, Operand,
    Place, Rvalue, Statement, Terminator,
};

/// One MIR invariant violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationError {
    /// Function has no return local or no entry block.
    EmptyFunction {
        /// Invalid function.
        function: DefId,
    },
    /// A block has no terminator.
    MissingTerminator {
        /// Invalid function.
        function: DefId,
        /// Unterminated block.
        block: BlockId,
    },
    /// A control-flow edge leaves the function.
    InvalidBlockTarget {
        /// Invalid function.
        function: DefId,
        /// Source block.
        block: BlockId,
        /// Out-of-range target.
        target: BlockId,
    },
    /// A local index is outside the local table.
    InvalidLocal {
        /// Invalid function.
        function: DefId,
        /// Referencing block.
        block: BlockId,
        /// Out-of-range local.
        local: LocalId,
    },
    /// A reachable path reads a local before all paths define it.
    UseBeforeDefinition {
        /// Invalid function.
        function: DefId,
        /// Referencing block.
        block: BlockId,
        /// Undefined local.
        local: LocalId,
    },
    /// MIR types disagree.
    TypeMismatch {
        /// Invalid function.
        function: DefId,
        /// Mismatched block.
        block: BlockId,
        /// Required type.
        expected: MirType,
        /// Observed type.
        actual: MirType,
    },
    /// Direct call targets no MIR function.
    UnknownFunction {
        /// Calling function.
        function: DefId,
        /// Calling block.
        block: BlockId,
        /// Missing callee.
        callee: DefId,
    },
    /// Direct call argument count differs from the signature.
    ArgumentCount {
        /// Calling function.
        function: DefId,
        /// Calling block.
        block: BlockId,
        /// Signature arity.
        expected: usize,
        /// Supplied arity.
        actual: usize,
    },
    /// Switch input is neither Boolean nor integer.
    InvalidSwitchType {
        /// Invalid function.
        function: DefId,
        /// Switching block.
        block: BlockId,
        /// Unsupported scrutinee type.
        actual: MirType,
    },
}

/// Proves structural, typing, target, and definite-initialization invariants
/// required by every Syllog backend.
///
/// # Errors
///
/// Returns all deterministically ordered verifier failures.
pub fn verify(program: &MirProgram) -> Result<(), Vec<VerificationError>> {
    let signatures = program
        .functions
        .iter()
        .map(|function| (function.id, function))
        .collect::<BTreeMap<_, _>>();
    let mut errors = Vec::new();
    for function in &program.functions {
        verify_function(function, &signatures, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn verify_function(
    function: &MirFunction,
    signatures: &BTreeMap<DefId, &MirFunction>,
    errors: &mut Vec<VerificationError>,
) {
    if function.locals.is_empty() || function.blocks.is_empty() {
        errors.push(VerificationError::EmptyFunction {
            function: function.id,
        });
        return;
    }
    if function.locals[0] != function.return_type {
        errors.push(VerificationError::TypeMismatch {
            function: function.id,
            block: BlockId(0),
            expected: function.return_type.clone(),
            actual: function.locals[0].clone(),
        });
    }
    let parameter_end = usize::try_from(function.parameter_count)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    if parameter_end > function.locals.len() {
        errors.push(VerificationError::InvalidLocal {
            function: function.id,
            block: BlockId(0),
            local: LocalId(function.parameter_count),
        });
        return;
    }
    verify_targets(function, signatures, errors);
    let incoming = definite_initialization(function);
    for (index, block) in function.blocks.iter().enumerate() {
        let block_id = BlockId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(mut defined) = incoming[index].clone() else {
            continue;
        };
        verify_block(function, block_id, block, signatures, &mut defined, errors);
    }
}

fn verify_targets(
    function: &MirFunction,
    signatures: &BTreeMap<DefId, &MirFunction>,
    errors: &mut Vec<VerificationError>,
) {
    for (index, block) in function.blocks.iter().enumerate() {
        let block_id = BlockId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(terminator) = &block.terminator else {
            errors.push(VerificationError::MissingTerminator {
                function: function.id,
                block: block_id,
            });
            continue;
        };
        for target in successors(terminator) {
            if usize::try_from(target.0).map_or(true, |target| target >= function.blocks.len()) {
                errors.push(VerificationError::InvalidBlockTarget {
                    function: function.id,
                    block: block_id,
                    target,
                });
            }
        }
        if let Terminator::Call {
            function: callee, ..
        } = terminator
            && !signatures.contains_key(callee)
        {
            errors.push(VerificationError::UnknownFunction {
                function: function.id,
                block: block_id,
                callee: *callee,
            });
        }
    }
}

fn definite_initialization(function: &MirFunction) -> Vec<Option<BTreeSet<LocalId>>> {
    let mut reachable = vec![false; function.blocks.len()];
    let mut queue = VecDeque::from([BlockId(0)]);
    while let Some(block) = queue.pop_front() {
        let Ok(index) = usize::try_from(block.0) else {
            continue;
        };
        if index >= function.blocks.len() || reachable[index] {
            continue;
        }
        reachable[index] = true;
        if let Some(terminator) = &function.blocks[index].terminator {
            queue.extend(successors(terminator));
        }
    }

    let universe = (0..function.locals.len())
        .filter_map(|index| u32::try_from(index).ok().map(LocalId))
        .collect::<BTreeSet<_>>();
    let parameters = (1..=function.parameter_count).map(LocalId).collect();
    let mut incoming = reachable
        .iter()
        .map(|is_reachable| is_reachable.then(|| universe.clone()))
        .collect::<Vec<_>>();
    incoming[0] = Some(parameters);

    let mut changed = true;
    while changed {
        changed = false;
        for (index, block) in function.blocks.iter().enumerate() {
            let Some(mut outgoing) = incoming[index].clone() else {
                continue;
            };
            for statement in &block.statements {
                if let Statement::Assign {
                    destination: Place::Local(local),
                    ..
                } = statement
                {
                    outgoing.insert(*local);
                }
            }
            if let Some(Terminator::Call {
                destination: Place::Local(local),
                ..
            }) = &block.terminator
            {
                outgoing.insert(*local);
            }
            if let Some(terminator) = &block.terminator {
                for successor in successors(terminator) {
                    let Ok(successor) = usize::try_from(successor.0) else {
                        continue;
                    };
                    if successor >= incoming.len() || !reachable[successor] || successor == 0 {
                        continue;
                    }
                    let merged = incoming[successor]
                        .as_ref()
                        .map(|current| current.intersection(&outgoing).copied().collect());
                    if merged != incoming[successor] {
                        incoming[successor] = merged;
                        changed = true;
                    }
                }
            }
        }
    }
    incoming
}

fn verify_block(
    function: &MirFunction,
    block_id: BlockId,
    block: &BasicBlock,
    signatures: &BTreeMap<DefId, &MirFunction>,
    defined: &mut BTreeSet<LocalId>,
    errors: &mut Vec<VerificationError>,
) {
    for statement in &block.statements {
        match statement {
            Statement::Assign { destination, value } => {
                let value_type = verify_rvalue(function, block_id, value, defined, errors);
                if let Some(destination_type) = place_type(function, block_id, destination, errors)
                {
                    if let Some(value_type) = value_type
                        && destination_type != value_type
                    {
                        errors.push(type_error(function, block_id, destination_type, value_type));
                    }
                    if let Place::Local(local) = destination {
                        defined.insert(*local);
                    }
                }
            }
            Statement::Drop(place) => {
                verify_local_use(function, block_id, place_local(place), defined, errors);
            }
        }
    }
    if let Some(terminator) = &block.terminator {
        verify_terminator(function, block_id, terminator, signatures, defined, errors);
    }
}

fn verify_terminator(
    function: &MirFunction,
    block: BlockId,
    terminator: &Terminator,
    signatures: &BTreeMap<DefId, &MirFunction>,
    defined: &BTreeSet<LocalId>,
    errors: &mut Vec<VerificationError>,
) {
    match terminator {
        Terminator::Return => verify_local_use(function, block, LocalId(0), defined, errors),
        Terminator::Goto(_) => {}
        Terminator::SwitchInt { value, .. } => {
            if let Some(actual) = operand_type(function, block, value, defined, errors)
                && !matches!(actual, MirType::Bool | MirType::I64 | MirType::U64)
            {
                errors.push(VerificationError::InvalidSwitchType {
                    function: function.id,
                    block,
                    actual,
                });
            }
        }
        Terminator::Call {
            function: callee,
            args,
            destination,
            ..
        } => {
            let Some(signature) = signatures.get(callee) else {
                return;
            };
            let parameter_count = usize::try_from(signature.parameter_count).unwrap_or(usize::MAX);
            if args.len() != parameter_count {
                errors.push(VerificationError::ArgumentCount {
                    function: function.id,
                    block,
                    expected: parameter_count,
                    actual: args.len(),
                });
            }
            for (index, argument) in args.iter().enumerate() {
                let actual = operand_type(function, block, argument, defined, errors);
                let expected = signature.locals.get(index + 1);
                if let (Some(actual), Some(expected)) = (actual, expected)
                    && actual != *expected
                {
                    errors.push(type_error(function, block, expected.clone(), actual));
                }
            }
            if let Some(actual) = place_type(function, block, destination, errors)
                && actual != signature.return_type
            {
                errors.push(type_error(
                    function,
                    block,
                    signature.return_type.clone(),
                    actual,
                ));
            }
        }
    }
}

fn verify_rvalue(
    function: &MirFunction,
    block: BlockId,
    value: &Rvalue,
    defined: &BTreeSet<LocalId>,
    errors: &mut Vec<VerificationError>,
) -> Option<MirType> {
    match value {
        Rvalue::Use(operand) => operand_type(function, block, operand, defined, errors),
        Rvalue::Discriminant(operand) => {
            operand_type(function, block, operand, defined, errors);
            Some(MirType::U64)
        }
        Rvalue::Aggregate { ty, fields } => {
            for field in fields {
                operand_type(function, block, field, defined, errors);
            }
            Some(MirType::Aggregate(*ty))
        }
        Rvalue::Binary {
            operator,
            left,
            right,
        } => {
            let left = operand_type(function, block, left, defined, errors)?;
            let right = operand_type(function, block, right, defined, errors)?;
            if left != right {
                errors.push(type_error(function, block, left.clone(), right));
            }
            Some(match operator {
                BinaryOp::Equal | BinaryOp::Less => MirType::Bool,
                BinaryOp::And | BinaryOp::Or if left == MirType::Bool => MirType::Bool,
                BinaryOp::And | BinaryOp::Or => {
                    errors.push(type_error(function, block, MirType::Bool, left));
                    MirType::Bool
                }
                _ => left,
            })
        }
    }
}

fn operand_type(
    function: &MirFunction,
    block: BlockId,
    operand: &Operand,
    defined: &BTreeSet<LocalId>,
    errors: &mut Vec<VerificationError>,
) -> Option<MirType> {
    match operand {
        Operand::Constant(constant) => Some(constant.ty()),
        Operand::Copy(local) | Operand::Move(local) => {
            verify_local_use(function, block, *local, defined, errors);
            function.locals.get(usize::try_from(local.0).ok()?).cloned()
        }
    }
}

fn place_type(
    function: &MirFunction,
    block: BlockId,
    place: &Place,
    errors: &mut Vec<VerificationError>,
) -> Option<MirType> {
    let local = place_local(place);
    let Some(ty) = function.locals.get(usize::try_from(local.0).ok()?) else {
        errors.push(VerificationError::InvalidLocal {
            function: function.id,
            block,
            local,
        });
        return None;
    };
    match place {
        Place::Local(_) => Some(ty.clone()),
        Place::Field { .. } => None,
    }
}

fn verify_local_use(
    function: &MirFunction,
    block: BlockId,
    local: LocalId,
    defined: &BTreeSet<LocalId>,
    errors: &mut Vec<VerificationError>,
) {
    let valid = usize::try_from(local.0).is_ok_and(|index| index < function.locals.len());
    if !valid {
        errors.push(VerificationError::InvalidLocal {
            function: function.id,
            block,
            local,
        });
    } else if !defined.contains(&local) {
        errors.push(VerificationError::UseBeforeDefinition {
            function: function.id,
            block,
            local,
        });
    }
}

fn place_local(place: &Place) -> LocalId {
    match place {
        Place::Local(local) => *local,
        Place::Field { base, .. } => *base,
    }
}

fn successors(terminator: &Terminator) -> Vec<BlockId> {
    match terminator {
        Terminator::Return => Vec::new(),
        Terminator::Goto(target) => vec![*target],
        Terminator::SwitchInt {
            targets, otherwise, ..
        } => targets
            .iter()
            .map(|(_, target)| *target)
            .chain([*otherwise])
            .collect(),
        Terminator::Call { next, .. } => vec![*next],
    }
}

fn type_error(
    function: &MirFunction,
    block: BlockId,
    expected: MirType,
    actual: MirType,
) -> VerificationError {
    VerificationError::TypeMismatch {
        function: function.id,
        block,
        expected,
        actual,
    }
}
