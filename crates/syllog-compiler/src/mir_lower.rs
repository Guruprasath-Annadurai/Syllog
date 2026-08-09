//! Typed HIR to verified control-flow MIR lowering.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use syllog_ir::{
    BasicBlock, BinaryOp, BlockId, Constant, DefId as MirDefId, LocalId, MirFunction, MirProgram,
    MirType, Operand, Place, Rvalue, Statement, Terminator, verify,
};
use syllog_parser::{BinaryOperator, Diagnostic, Literal, Severity, Span};
use syllog_semantic::{PrimitiveType, ResolvedType};

use crate::hir::{
    DefId, HirBlock, HirDefinition, HirDefinitionKind, HirExprKind, HirPattern, HirProgram,
    HirStatement, TypedExpr,
};

/// Lowers all executable HIR functions into MIR and verifies the result before
/// returning it to interpreters or backends.
///
/// # Errors
///
/// Returns source diagnostics for unsupported executable types or expressions,
/// followed by invariant diagnostics if compiler-produced MIR fails verification.
pub fn lower_to_mir(hir: &HirProgram) -> Result<MirProgram, Vec<Diagnostic>> {
    let index = ProgramIndex::new(hir);
    let mut functions = Vec::new();
    let mut diagnostics = Vec::new();
    for module in &hir.modules {
        for definition in &module.definitions {
            if let HirDefinitionKind::Function(function) = &definition.kind {
                let mut builder = FunctionBuilder::new(definition, function, &index);
                let lowered = builder.lower();
                diagnostics.append(&mut builder.diagnostics);
                functions.push(lowered);
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let mut program = MirProgram {
        functions,
        entry: hir.entry.map(mir_def_id),
    };
    insert_drops(&mut program);
    if let Err(errors) = verify(&program) {
        return Err(errors
            .into_iter()
            .map(|error| mir_diagnostic(Span::default(), "SYL3102", format!("{error:?}")))
            .collect());
    }
    Ok(program)
}

fn insert_drops(program: &mut MirProgram) {
    for function in &mut program.functions {
        insert_function_drops(function);
    }
}

fn insert_function_drops(function: &mut MirFunction) {
    if function.blocks.is_empty() {
        return;
    }
    let reachable = mir_reachable(function);
    let universe = (0..function.locals.len())
        .filter_map(|index| u32::try_from(index).ok().map(LocalId))
        .collect::<BTreeSet<_>>();
    let parameters = (1..=function.parameter_count).map(LocalId).collect();
    let mut incoming = reachable
        .iter()
        .map(|reachable| reachable.then(|| universe.clone()))
        .collect::<Vec<_>>();
    incoming[0] = Some(parameters);
    let mut changed = true;
    while changed {
        changed = false;
        for (index, block) in function.blocks.iter().enumerate() {
            let Some(mut state) = incoming[index].clone() else {
                continue;
            };
            mir_transfer(block, &mut state);
            if let Some(terminator) = &block.terminator {
                for successor in mir_successors(terminator) {
                    let Ok(successor) = usize::try_from(successor.0) else {
                        continue;
                    };
                    if successor >= incoming.len() || !reachable[successor] {
                        continue;
                    }
                    let merged = incoming[successor]
                        .as_ref()
                        .map(|current| current.intersection(&state).copied().collect());
                    if merged != incoming[successor] {
                        incoming[successor] = merged;
                        changed = true;
                    }
                }
            }
        }
    }
    let local_types = function.locals.clone();
    for (index, block) in function.blocks.iter_mut().enumerate() {
        let Some(mut state) = incoming[index].clone() else {
            continue;
        };
        mir_transfer_statements(&block.statements, &mut state);
        let drop_candidates = match block.terminator.as_ref() {
            Some(Terminator::Return) => state,
            Some(Terminator::Goto(target)) => usize::try_from(target.0)
                .ok()
                .and_then(|target| incoming.get(target))
                .and_then(Clone::clone)
                .map_or_else(BTreeSet::new, |next| {
                    state.difference(&next).copied().collect()
                }),
            _ => BTreeSet::new(),
        };
        for local in drop_candidates.into_iter().rev() {
            if local != LocalId(0)
                && local_types
                    .get(usize::try_from(local.0).unwrap_or(usize::MAX))
                    .is_some_and(mir_type_needs_drop)
            {
                block.statements.push(Statement::Drop(Place::Local(local)));
            }
        }
    }
}

fn mir_reachable(function: &MirFunction) -> Vec<bool> {
    let mut reachable = vec![false; function.blocks.len()];
    let mut queue = VecDeque::from([BlockId(0)]);
    while let Some(block) = queue.pop_front() {
        let Ok(index) = usize::try_from(block.0) else {
            continue;
        };
        if index >= reachable.len() || reachable[index] {
            continue;
        }
        reachable[index] = true;
        if let Some(terminator) = &function.blocks[index].terminator {
            queue.extend(mir_successors(terminator));
        }
    }
    reachable
}

fn mir_transfer(block: &BasicBlock, state: &mut BTreeSet<LocalId>) {
    mir_transfer_statements(&block.statements, state);
    if let Some(terminator) = &block.terminator {
        match terminator {
            Terminator::SwitchInt { value, .. } => mir_consume_operand(value, state),
            Terminator::Call {
                args, destination, ..
            } => {
                for argument in args {
                    mir_consume_operand(argument, state);
                }
                if let Place::Local(local) = destination {
                    state.insert(*local);
                }
            }
            Terminator::Return | Terminator::Goto(_) => {}
        }
    }
}

fn mir_transfer_statements(statements: &[Statement], state: &mut BTreeSet<LocalId>) {
    for statement in statements {
        match statement {
            Statement::Assign { destination, value } => {
                match value {
                    Rvalue::Use(operand) | Rvalue::Discriminant(operand) => {
                        mir_consume_operand(operand, state);
                    }
                    Rvalue::Aggregate { fields, .. } => {
                        for field in fields {
                            mir_consume_operand(field, state);
                        }
                    }
                    Rvalue::Binary { left, right, .. } => {
                        mir_consume_operand(left, state);
                        mir_consume_operand(right, state);
                    }
                    Rvalue::Borrow { .. } => {}
                }
                if let Place::Local(local) = destination {
                    state.insert(*local);
                }
            }
            Statement::Drop(place) => {
                state.remove(&mir_place_local(place));
            }
        }
    }
}

fn mir_consume_operand(operand: &Operand, state: &mut BTreeSet<LocalId>) {
    if let Operand::Move(local) = operand {
        state.remove(local);
    }
}

fn mir_place_local(place: &Place) -> LocalId {
    match place {
        Place::Local(local) => *local,
        Place::Field { base, .. } => *base,
    }
}

fn mir_successors(terminator: &Terminator) -> Vec<BlockId> {
    match terminator {
        Terminator::Return => Vec::new(),
        Terminator::Goto(target) => vec![*target],
        Terminator::SwitchInt {
            targets, otherwise, ..
        } => targets
            .iter()
            .map(|(_, target)| *target)
            .chain(std::iter::once(*otherwise))
            .collect(),
        Terminator::Call { next, .. } => vec![*next],
    }
}

fn mir_type_needs_drop(ty: &MirType) -> bool {
    matches!(ty, MirType::String | MirType::Aggregate(_))
        || matches!(ty, MirType::Reference { mutable: true, .. })
}

struct ProgramIndex {
    types: BTreeMap<String, MirDefId>,
    variants: BTreeMap<DefId, (MirDefId, u64)>,
}

impl ProgramIndex {
    fn new(program: &HirProgram) -> Self {
        let definitions = program
            .modules
            .iter()
            .flat_map(|module| &module.definitions)
            .map(|definition| (definition.id, definition))
            .collect::<BTreeMap<_, _>>();
        let mut types = BTreeMap::new();
        let mut variants = BTreeMap::new();
        for definition in definitions.values() {
            if matches!(
                definition.kind,
                HirDefinitionKind::Struct { .. }
                    | HirDefinitionKind::Enum { .. }
                    | HirDefinitionKind::State { .. }
            ) {
                types.insert(definition.name.clone(), mir_def_id(definition.id));
            }
            if let HirDefinitionKind::Enum { variants: declared } = &definition.kind {
                for (discriminant, variant) in declared.iter().enumerate() {
                    variants.insert(
                        variant.id,
                        (
                            mir_def_id(definition.id),
                            u64::try_from(discriminant).unwrap_or(u64::MAX),
                        ),
                    );
                }
            }
        }
        Self { types, variants }
    }

    fn lower_type(&self, ty: &ResolvedType) -> Option<MirType> {
        match ty {
            ResolvedType::Unit => Some(MirType::Unit),
            ResolvedType::Primitive(PrimitiveType::Bool) => Some(MirType::Bool),
            ResolvedType::Primitive(PrimitiveType::String | PrimitiveType::Str) => {
                Some(MirType::String)
            }
            ResolvedType::Primitive(PrimitiveType::Signed(width)) if width == "I64" => {
                Some(MirType::I64)
            }
            ResolvedType::Primitive(PrimitiveType::Unsigned(width)) if width == "U64" => {
                Some(MirType::U64)
            }
            ResolvedType::Reference { mutable, inner, .. } => Some(MirType::Reference {
                mutable: *mutable,
                inner: Box::new(self.lower_type(inner)?),
            }),
            ResolvedType::Struct(name) | ResolvedType::Enum(name) | ResolvedType::State(name) => {
                self.types.get(name).copied().map(MirType::Aggregate)
            }
            _ => None,
        }
    }
}

struct FunctionBuilder<'a> {
    definition: &'a HirDefinition,
    function: &'a crate::hir::HirFunction,
    index: &'a ProgramIndex,
    locals: Vec<MirType>,
    blocks: Vec<BasicBlock>,
    current: BlockId,
    bindings: BTreeMap<DefId, LocalId>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> FunctionBuilder<'a> {
    fn new(
        definition: &'a HirDefinition,
        function: &'a crate::hir::HirFunction,
        index: &'a ProgramIndex,
    ) -> Self {
        Self {
            definition,
            function,
            index,
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                statements: Vec::new(),
                terminator: None,
            }],
            current: BlockId(0),
            bindings: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn lower(&mut self) -> MirFunction {
        let result = self.required_type(&self.function.result, self.definition.span);
        self.locals.push(result.clone());
        for parameter in &self.function.parameters {
            let ty = self.required_type(&parameter.ty, parameter.span);
            let local = self.new_local(ty);
            self.bindings.insert(parameter.id, local);
        }
        self.lower_block(&self.function.body, &result);
        if self.current_block().terminator.is_none() {
            if result == MirType::Unit {
                self.assign(
                    Place::Local(LocalId(0)),
                    Rvalue::Use(Operand::Constant(Constant::Unit)),
                );
            }
            self.terminate(Terminator::Return);
        }
        MirFunction {
            id: mir_def_id(self.definition.id),
            parameter_count: u32::try_from(self.function.parameters.len()).unwrap_or(u32::MAX),
            return_type: result,
            locals: std::mem::take(&mut self.locals),
            blocks: std::mem::take(&mut self.blocks),
        }
    }

    fn lower_block(&mut self, block: &HirBlock, result: &MirType) {
        let last = block.statements.len().saturating_sub(1);
        for (index, statement) in block.statements.iter().enumerate() {
            if self.current_block().terminator.is_some() {
                break;
            }
            match statement {
                HirStatement::Let {
                    definition,
                    ty,
                    value,
                    ..
                } => {
                    let ty = self.required_type(ty, value.span);
                    let operand = self.lower_expression(value, Some(&ty));
                    let local = self.new_local(ty);
                    self.assign(Place::Local(local), Rvalue::Use(operand));
                    self.bindings.insert(*definition, local);
                }
                HirStatement::Return(value) => {
                    if let Some(value) = value {
                        let operand = self.lower_expression(value, Some(result));
                        self.assign(Place::Local(LocalId(0)), Rvalue::Use(operand));
                    } else if *result == MirType::Unit {
                        self.assign(
                            Place::Local(LocalId(0)),
                            Rvalue::Use(Operand::Constant(Constant::Unit)),
                        );
                    }
                    self.terminate(Terminator::Return);
                }
                HirStatement::Expression(expression) => {
                    let is_result = index == last;
                    let operand = self.lower_expression(expression, is_result.then_some(result));
                    if is_result {
                        self.assign(Place::Local(LocalId(0)), Rvalue::Use(operand));
                        self.terminate(Terminator::Return);
                    }
                }
            }
        }
    }

    fn lower_expression(&mut self, expression: &TypedExpr, expected: Option<&MirType>) -> Operand {
        match &expression.kind {
            HirExprKind::Borrow { mutable, operand } => {
                let HirExprKind::Reference { definition } = operand.kind else {
                    self.error(expression.span, "MIR borrows require a direct local place");
                    return Operand::Constant(Constant::Unit);
                };
                let Some(local) = self.bindings.get(&definition).copied() else {
                    self.error(expression.span, "MIR borrow target is not a local");
                    return Operand::Constant(Constant::Unit);
                };
                let ty = self
                    .index
                    .lower_type(&expression.ty)
                    .unwrap_or(MirType::Unit);
                let reference = self.new_local(ty);
                self.assign(
                    Place::Local(reference),
                    Rvalue::Borrow {
                        mutable: *mutable,
                        place: Place::Local(local),
                    },
                );
                Operand::Move(reference)
            }
            HirExprKind::Await(operand) => self.lower_expression(operand, expected),
            HirExprKind::Literal(literal) => self.lower_literal(literal, expression, expected),
            HirExprKind::Reference { definition } => {
                self.lower_reference(*definition, expression.span)
            }
            HirExprKind::Binary {
                operator,
                left,
                right,
            } => self.lower_binary_expression(*operator, left, right, expected, expression.span),
            HirExprKind::Call { callee, arguments } => {
                self.lower_call(callee, arguments, expression, expected)
            }
            HirExprKind::Match { value, arms } => {
                self.lower_match(expression, value, arms, expected)
            }
            HirExprKind::Block(block) => {
                let result = expected.cloned().unwrap_or(MirType::Unit);
                self.lower_block(block, &result);
                Operand::Copy(LocalId(0))
            }
            HirExprKind::Array(_) | HirExprKind::Field { .. } | HirExprKind::Unary { .. } => {
                self.error(
                    expression.span,
                    "expression is not in the executable MIR subset",
                );
                Operand::Constant(Constant::Unit)
            }
        }
    }

    fn lower_reference(&mut self, definition: DefId, span: Span) -> Operand {
        if let Some(local) = self.bindings.get(&definition) {
            if self
                .locals
                .get(usize::try_from(local.0).unwrap_or(usize::MAX))
                .is_some_and(mir_type_is_copy)
            {
                Operand::Copy(*local)
            } else {
                Operand::Move(*local)
            }
        } else if let Some((aggregate, discriminant)) = self.index.variants.get(&definition) {
            let aggregate = *aggregate;
            let discriminant = *discriminant;
            let local = self.new_local(MirType::Aggregate(aggregate));
            self.assign(
                Place::Local(local),
                Rvalue::Aggregate {
                    ty: aggregate,
                    discriminant,
                    fields: Vec::new(),
                },
            );
            Operand::Move(local)
        } else {
            self.error(span, "value reference is not executable in MIR");
            Operand::Constant(Constant::Unit)
        }
    }

    fn lower_binary_expression(
        &mut self,
        operator: BinaryOperator,
        left: &TypedExpr,
        right: &TypedExpr,
        expected: Option<&MirType>,
        span: Span,
    ) -> Operand {
        let left = self.lower_expression(left, expected);
        let operand_type = self.operand_type(&left).or_else(|| expected.cloned());
        let right = self.lower_expression(right, operand_type.as_ref());
        let result = if matches!(
            operator,
            BinaryOperator::Equal
                | BinaryOperator::NotEqual
                | BinaryOperator::Less
                | BinaryOperator::LessEqual
                | BinaryOperator::Greater
                | BinaryOperator::GreaterEqual
        ) {
            MirType::Bool
        } else {
            operand_type.unwrap_or(MirType::Unit)
        };
        let Some(operator) = lower_binary(operator) else {
            self.error(span, "binary operator is not in the executable subset");
            return Operand::Constant(Constant::Unit);
        };
        let local = self.new_local(result);
        self.assign(
            Place::Local(local),
            Rvalue::Binary {
                operator,
                left,
                right,
            },
        );
        Operand::Move(local)
    }

    fn lower_call(
        &mut self,
        callee: &TypedExpr,
        arguments: &[TypedExpr],
        expression: &TypedExpr,
        expected: Option<&MirType>,
    ) -> Operand {
        let HirExprKind::Reference { definition: callee } = callee.kind else {
            self.error(expression.span, "only direct function calls are executable");
            return Operand::Constant(Constant::Unit);
        };
        let args = arguments
            .iter()
            .map(|argument| self.lower_expression(argument, None))
            .collect();
        let result = expected.cloned().unwrap_or_else(|| {
            self.index
                .lower_type(&expression.ty)
                .unwrap_or(MirType::Unit)
        });
        let destination = self.new_local(result);
        let next = self.new_block();
        self.terminate(Terminator::Call {
            function: mir_def_id(callee),
            args,
            destination: Place::Local(destination),
            next,
        });
        self.current = next;
        Operand::Move(destination)
    }

    fn lower_match(
        &mut self,
        expression: &TypedExpr,
        value: &TypedExpr,
        arms: &[crate::hir::HirMatchArm],
        expected: Option<&MirType>,
    ) -> Operand {
        if arms.is_empty() {
            self.error(expression.span, "match requires at least one arm");
            return Operand::Constant(Constant::Unit);
        }
        let value = self.lower_expression(value, None);
        let discriminant = self.new_local(MirType::U64);
        self.assign(Place::Local(discriminant), Rvalue::Discriminant(value));
        let result_type = expected.cloned().unwrap_or_else(|| {
            self.index
                .lower_type(&expression.ty)
                .unwrap_or(MirType::Unit)
        });
        let result = self.new_local(result_type.clone());
        let join = self.new_block();
        let arm_blocks = arms.iter().map(|_| self.new_block()).collect::<Vec<_>>();
        let mut targets = Vec::new();
        for (arm, block) in arms.iter().zip(&arm_blocks).take(arms.len() - 1) {
            let Some(discriminant) = self.pattern_discriminant(&arm.pattern) else {
                self.error(
                    arm.span,
                    "non-final match arms require a closed discriminant",
                );
                continue;
            };
            targets.push((u128::from(discriminant), *block));
        }
        self.terminate(Terminator::SwitchInt {
            value: Operand::Copy(discriminant),
            targets,
            otherwise: *arm_blocks.last().expect("non-empty arms"),
        });
        for (arm, block) in arms.iter().zip(arm_blocks) {
            self.current = block;
            if arm.guard.is_some() {
                self.error(arm.span, "guarded matches are not in the executable subset");
            }
            let value = self.lower_expression(&arm.body, Some(&result_type));
            self.assign(Place::Local(result), Rvalue::Use(value));
            self.terminate(Terminator::Goto(join));
        }
        self.current = join;
        Operand::Move(result)
    }

    fn pattern_discriminant(&mut self, pattern: &HirPattern) -> Option<u64> {
        match pattern {
            HirPattern::Variant { definition, fields } if fields.is_empty() => {
                self.index.variants.get(definition).map(|(_, value)| *value)
            }
            HirPattern::Literal(Literal::Boolean(value)) => Some(u64::from(*value)),
            _ => None,
        }
    }

    fn lower_literal(
        &mut self,
        literal: &Literal,
        expression: &TypedExpr,
        expected: Option<&MirType>,
    ) -> Operand {
        let constant = match literal {
            Literal::String(value) => Constant::String(value.clone()),
            Literal::Boolean(value) => Constant::Bool(*value),
            Literal::Integer(value) if expected == Some(&MirType::I64) => Constant::I64(*value),
            Literal::Integer(value) => match u64::try_from(*value) {
                Ok(value) => Constant::U64(value),
                Err(_) => Constant::I64(*value),
            },
            Literal::Float(_) | Literal::Identifier(_) => {
                self.error(
                    expression.span,
                    "literal is not in the executable MIR subset",
                );
                Constant::Unit
            }
        };
        Operand::Constant(constant)
    }

    fn required_type(&mut self, ty: &ResolvedType, span: Span) -> MirType {
        self.index.lower_type(ty).unwrap_or_else(|| {
            self.error(
                span,
                format!("type '{ty}' is not in the executable MIR subset"),
            );
            MirType::Unit
        })
    }

    fn operand_type(&self, operand: &Operand) -> Option<MirType> {
        match operand {
            Operand::Constant(constant) => Some(constant.ty()),
            Operand::Copy(local) | Operand::Move(local) => {
                self.locals.get(usize::try_from(local.0).ok()?).cloned()
            }
        }
    }

    fn new_local(&mut self, ty: MirType) -> LocalId {
        let local = LocalId(u32::try_from(self.locals.len()).unwrap_or(u32::MAX));
        self.locals.push(ty);
        local
    }

    fn new_block(&mut self) -> BlockId {
        let block = BlockId(u32::try_from(self.blocks.len()).unwrap_or(u32::MAX));
        self.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        block
    }

    fn current_block(&self) -> &BasicBlock {
        &self.blocks[usize::try_from(self.current.0).expect("block identity fits usize")]
    }

    fn current_block_mut(&mut self) -> &mut BasicBlock {
        &mut self.blocks[usize::try_from(self.current.0).expect("block identity fits usize")]
    }

    fn assign(&mut self, destination: Place, value: Rvalue) {
        self.current_block_mut()
            .statements
            .push(Statement::Assign { destination, value });
    }

    fn terminate(&mut self, terminator: Terminator) {
        self.current_block_mut().terminator = Some(terminator);
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(mir_diagnostic(span, "SYL3101", message));
    }
}

fn mir_type_is_copy(ty: &MirType) -> bool {
    matches!(
        ty,
        MirType::Unit | MirType::Bool | MirType::I64 | MirType::U64
    ) || matches!(ty, MirType::Reference { mutable: false, .. })
}

fn lower_binary(operator: BinaryOperator) -> Option<BinaryOp> {
    Some(match operator {
        BinaryOperator::Add => BinaryOp::Add,
        BinaryOperator::Subtract => BinaryOp::Subtract,
        BinaryOperator::Multiply => BinaryOp::Multiply,
        BinaryOperator::Divide => BinaryOp::Divide,
        BinaryOperator::Equal => BinaryOp::Equal,
        BinaryOperator::Less => BinaryOp::Less,
        BinaryOperator::And => BinaryOp::And,
        BinaryOperator::Or => BinaryOp::Or,
        _ => return None,
    })
}

fn mir_def_id(id: DefId) -> MirDefId {
    MirDefId {
        module: id.module.0,
        index: id.index,
    }
}

fn mir_diagnostic(span: Span, code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: Severity::Error,
        message: message.into(),
        file: "<mir>".into(),
        span,
    }
}
