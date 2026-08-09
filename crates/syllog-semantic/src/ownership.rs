//! Source-level affine ownership, lexical borrowing, and region checks.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use syllog_parser::{
    Ast, Block, Diagnostic, Expr, ExprKind, FunctionNode, Item, Literal, MatchArm, Severity, Span,
    StatementKind, TypeKind, TypeNode,
};

/// Observable ownership state used by diagnostics and tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnershipState {
    /// Value may be moved, read, or borrowed.
    Available,
    /// Affine value has been consumed.
    Moved,
    /// One or more shared references are live.
    SharedBorrowed(u32),
    /// One exclusive mutable reference is live.
    MutBorrowed,
}

/// Result of the ownership and region pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipAnalysis {
    /// Deterministically ordered ownership diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Parameter,
    Local,
}

#[derive(Debug, Clone)]
struct Variable {
    state: OwnershipState,
    copy: bool,
    origin: Origin,
    borrow: Option<(String, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseMode {
    Consume,
    Read,
}

/// Checks every function for affine moves, aliasing, and escaping references.
#[must_use]
pub fn analyze_ownership(file: &str, ast: &Ast) -> OwnershipAnalysis {
    let mut diagnostics = Vec::new();
    for item in &ast.items {
        if let Item::Function(function) = item {
            FunctionChecker::new(file, function, &mut diagnostics).check();
        }
    }
    OwnershipAnalysis { diagnostics }
}

struct FunctionChecker<'a> {
    file: &'a str,
    function: &'a FunctionNode,
    diagnostics: &'a mut Vec<Diagnostic>,
    variables: BTreeMap<String, Variable>,
}

impl<'a> FunctionChecker<'a> {
    fn new(
        file: &'a str,
        function: &'a FunctionNode,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        let variables = function
            .parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    Variable {
                        state: OwnershipState::Available,
                        copy: type_is_copy(&parameter.ty),
                        origin: Origin::Parameter,
                        borrow: None,
                    },
                )
            })
            .collect();
        Self {
            file,
            function,
            diagnostics,
            variables,
        }
    }

    fn check(mut self) {
        self.check_signature_regions();
        self.check_block(&self.function.body);
    }

    fn check_signature_regions(&mut self) {
        let Some(TypeNode {
            kind:
                TypeKind::Reference {
                    lifetime: result_region,
                    ..
                },
            span,
        }) = self.function.return_type.as_ref()
        else {
            return;
        };
        let sources = self
            .function
            .parameters
            .iter()
            .filter_map(|parameter| match &parameter.ty.kind {
                TypeKind::Reference { lifetime, .. } => Some(lifetime.as_deref()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let valid = match result_region.as_deref() {
            Some(region) => sources.contains(&Some(region)),
            None => sources.len() == 1,
        };
        if !valid {
            self.error(
                "SYL2604",
                "returned reference lifetime is not tied to exactly one input reference",
                *span,
            );
        }
    }

    fn check_block(&mut self, block: &Block) {
        let mut shadowed = BTreeMap::<String, Option<Variable>>::new();
        let last_uses = block_last_uses(block);
        for (index, statement) in block.statements.iter().enumerate() {
            match &statement.kind {
                StatementKind::Let { name, ty, value } => {
                    self.check_expr(value, UseMode::Consume);
                    let borrow = borrow_target(value);
                    let copy = ty
                        .as_ref()
                        .map_or_else(|| expression_is_copy(value, &self.variables), type_is_copy);
                    shadowed
                        .entry(name.clone())
                        .or_insert_with(|| self.variables.get(name).cloned());
                    self.release_borrow(name);
                    self.variables.insert(
                        name.clone(),
                        Variable {
                            state: OwnershipState::Available,
                            copy,
                            origin: Origin::Local,
                            borrow,
                        },
                    );
                }
                StatementKind::Return(value) => {
                    if let Some(value) = value {
                        self.check_return_escape(value);
                        self.check_expr(value, UseMode::Consume);
                    }
                }
                StatementKind::Expression(expression) => {
                    self.check_expr(expression, UseMode::Consume);
                }
            }
            let ending = self
                .variables
                .iter()
                .filter(|(name, variable)| {
                    variable.borrow.is_some()
                        && last_uses.get(*name).copied().unwrap_or(index) <= index
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            for name in ending {
                self.release_borrow(&name);
            }
        }
        for (name, previous) in shadowed {
            self.release_borrow(&name);
            match previous {
                Some(variable) => {
                    self.variables.insert(name, variable);
                }
                None => {
                    self.variables.remove(&name);
                }
            }
        }
    }

    fn check_expr(&mut self, expression: &Expr, mode: UseMode) {
        match &expression.kind {
            ExprKind::Borrow { mutable, operand } => self.borrow(operand, *mutable),
            ExprKind::Path(path) if path.len() == 1 => {
                self.use_variable(&path[0], mode, expression.span);
            }
            ExprKind::Literal(Literal::Identifier(name)) => {
                self.use_variable(name, mode, expression.span);
            }
            ExprKind::Array(items) => {
                for item in items {
                    self.check_expr(item, UseMode::Consume);
                }
            }
            ExprKind::Call { callee, arguments } => {
                self.check_expr(callee, UseMode::Read);
                for argument in arguments {
                    self.check_expr(&argument.value, UseMode::Consume);
                }
            }
            ExprKind::Field { base, .. } => self.check_expr(base, mode),
            ExprKind::Unary { operand, .. } => self.check_expr(operand, mode),
            ExprKind::Await(operand) => {
                let before = self.borrowed_targets();
                self.check_expr(operand, mode);
                let after = self.borrowed_targets();
                for name in before
                    .into_iter()
                    .chain(after)
                    .collect::<std::collections::BTreeSet<_>>()
                {
                    self.error(
                        "SYL2606",
                        format!(
                            "borrow of '{name}' crosses an await suspension; move owned data into the task or end the borrow first"
                        ),
                        expression.span,
                    );
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.check_expr(left, UseMode::Read);
                self.check_expr(right, UseMode::Read);
            }
            ExprKind::Match { value, arms } => {
                self.check_expr(value, UseMode::Consume);
                self.check_match_arms(arms);
            }
            ExprKind::Block(block) => self.check_block(block),
            ExprKind::Literal(_) | ExprKind::Path(_) => {}
        }
    }

    fn check_match_arms(&mut self, arms: &[MatchArm]) {
        let before = self.variables.clone();
        let mut branches = Vec::new();
        for arm in arms {
            self.variables = before.clone();
            if let Some(guard) = &arm.guard {
                self.check_expr(guard, UseMode::Read);
            }
            self.check_expr(&arm.body, UseMode::Consume);
            branches.push(self.variables.clone());
        }
        self.variables = before;
        for (name, variable) in &mut self.variables {
            let states = branches
                .iter()
                .filter_map(|branch| branch.get(name).map(|value| value.state))
                .collect::<Vec<_>>();
            if states.contains(&OwnershipState::Moved) {
                variable.state = OwnershipState::Moved;
            } else if states.contains(&OwnershipState::MutBorrowed) {
                variable.state = OwnershipState::MutBorrowed;
            } else {
                let shared = states
                    .iter()
                    .filter_map(|state| match state {
                        OwnershipState::SharedBorrowed(count) => Some(*count),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                variable.state = if shared == 0 {
                    OwnershipState::Available
                } else {
                    OwnershipState::SharedBorrowed(shared)
                };
            }
        }
    }

    fn use_variable(&mut self, name: &str, mode: UseMode, span: Span) {
        let Some(variable) = self.variables.get(name).cloned() else {
            return;
        };
        if variable.state == OwnershipState::Moved {
            self.error("SYL2602", format!("use of moved value '{name}'"), span);
            return;
        }
        if mode == UseMode::Consume && !variable.copy {
            if matches!(
                variable.state,
                OwnershipState::SharedBorrowed(_) | OwnershipState::MutBorrowed
            ) {
                self.error(
                    "SYL2605",
                    format!("cannot move '{name}' while it is borrowed"),
                    span,
                );
            } else if let Some(current) = self.variables.get_mut(name) {
                current.state = OwnershipState::Moved;
            }
        }
    }

    fn borrow(&mut self, operand: &Expr, mutable: bool) {
        let Some(name) = place_name(operand) else {
            return;
        };
        let Some(variable) = self.variables.get(name).cloned() else {
            return;
        };
        match (mutable, variable.state) {
            (_, OwnershipState::Moved) => self.error(
                "SYL2602",
                format!("cannot borrow moved value '{name}'"),
                operand.span,
            ),
            (true, OwnershipState::Available) => {
                self.variables.get_mut(name).expect("known variable").state =
                    OwnershipState::MutBorrowed;
            }
            (false, OwnershipState::Available) => {
                self.variables.get_mut(name).expect("known variable").state =
                    OwnershipState::SharedBorrowed(1);
            }
            (false, OwnershipState::SharedBorrowed(count)) => {
                self.variables.get_mut(name).expect("known variable").state =
                    OwnershipState::SharedBorrowed(count.saturating_add(1));
            }
            _ => self.error(
                "SYL2603",
                format!("borrow of '{name}' conflicts with an overlapping borrow"),
                operand.span,
            ),
        }
    }

    fn release_borrow(&mut self, binding: &str) {
        let borrow = self
            .variables
            .get(binding)
            .and_then(|value| value.borrow.clone());
        let Some((target, mutable)) = borrow else {
            return;
        };
        if let Some(target) = self.variables.get_mut(&target) {
            target.state = match (mutable, target.state) {
                (true, OwnershipState::MutBorrowed)
                | (false, OwnershipState::SharedBorrowed(1)) => OwnershipState::Available,
                (false, OwnershipState::SharedBorrowed(count)) => {
                    OwnershipState::SharedBorrowed(count.saturating_sub(1))
                }
                (_, state) => state,
            };
        }
        if let Some(binding) = self.variables.get_mut(binding) {
            binding.borrow = None;
        }
    }

    fn borrowed_targets(&self) -> Vec<String> {
        self.variables
            .iter()
            .filter(|(_, variable)| {
                matches!(
                    variable.state,
                    OwnershipState::SharedBorrowed(_) | OwnershipState::MutBorrowed
                )
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn check_return_escape(&mut self, expression: &Expr) {
        let target = match &expression.kind {
            ExprKind::Borrow { operand, .. } => place_name(operand),
            ExprKind::Path(path) if path.len() == 1 => self
                .variables
                .get(&path[0])
                .and_then(|variable| variable.borrow.as_ref().map(|borrow| borrow.0.as_str())),
            _ => None,
        };
        if let Some(target) = target
            && self
                .variables
                .get(target)
                .is_some_and(|variable| variable.origin == Origin::Local)
        {
            self.error(
                "SYL2604",
                format!("reference to local '{target}' escapes its region"),
                expression.span,
            );
        }
    }

    fn error(&mut self, code: &str, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            file: self.file.into(),
            span,
        });
    }
}

fn place_name(expression: &Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Path(path) if path.len() == 1 => Some(&path[0]),
        ExprKind::Literal(Literal::Identifier(name)) => Some(name),
        // Until field-sensitive move paths land, borrowing a projection
        // conservatively borrows its entire root aggregate.
        ExprKind::Field { base, .. } => place_name(base),
        _ => None,
    }
}

fn borrow_target(expression: &Expr) -> Option<(String, bool)> {
    match &expression.kind {
        ExprKind::Borrow { mutable, operand } => {
            place_name(operand).map(|name| (name.to_owned(), *mutable))
        }
        _ => None,
    }
}

fn type_is_copy(ty: &TypeNode) -> bool {
    match &ty.kind {
        TypeKind::Reference { mutable, .. } => !mutable,
        TypeKind::Tuple(items) => items.iter().all(type_is_copy),
        TypeKind::Path {
            segments,
            arguments,
        } if segments.len() == 1 && arguments.is_empty() => {
            let name = segments[0].as_str();
            matches!(name, "Bool" | "Char" | "Duration" | "Size" | "Str")
                || name
                    .strip_prefix(['I', 'U', 'F'])
                    .is_some_and(|width| width.chars().all(|character| character.is_ascii_digit()))
        }
        TypeKind::Array(_) | TypeKind::Path { .. } => false,
    }
}

fn expression_is_copy(expression: &Expr, variables: &BTreeMap<String, Variable>) -> bool {
    match &expression.kind {
        ExprKind::Literal(Literal::String(_)) | ExprKind::Array(_) => false,
        ExprKind::Borrow { mutable, .. } => !mutable,
        ExprKind::Path(path) if path.len() == 1 => {
            variables.get(&path[0]).is_none_or(|variable| variable.copy)
        }
        ExprKind::Literal(Literal::Identifier(name)) => {
            variables.get(name).is_none_or(|variable| variable.copy)
        }
        _ => true,
    }
}

fn block_last_uses(block: &Block) -> BTreeMap<String, usize> {
    let mut uses = BTreeMap::new();
    for (index, statement) in block.statements.iter().enumerate() {
        let expression = match &statement.kind {
            StatementKind::Let { value, .. } | StatementKind::Expression(value) => Some(value),
            StatementKind::Return(value) => value.as_ref(),
        };
        if let Some(expression) = expression {
            collect_uses(expression, index, &mut uses);
        }
    }
    uses
}

fn collect_uses(expression: &Expr, index: usize, uses: &mut BTreeMap<String, usize>) {
    if let Some(name) = place_name(expression) {
        uses.insert(name.to_owned(), index);
    }
    match &expression.kind {
        ExprKind::Borrow { operand, .. }
        | ExprKind::Await(operand)
        | ExprKind::Unary { operand, .. }
        | ExprKind::Field { base: operand, .. } => collect_uses(operand, index, uses),
        ExprKind::Array(items) => {
            for item in items {
                collect_uses(item, index, uses);
            }
        }
        ExprKind::Call { callee, arguments } => {
            collect_uses(callee, index, uses);
            for argument in arguments {
                collect_uses(&argument.value, index, uses);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            collect_uses(left, index, uses);
            collect_uses(right, index, uses);
        }
        ExprKind::Match { value, arms } => {
            collect_uses(value, index, uses);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_uses(guard, index, uses);
                }
                collect_uses(&arm.body, index, uses);
            }
        }
        ExprKind::Block(block) => {
            for name in block_last_uses(block).keys() {
                uses.insert(name.clone(), index);
            }
        }
        ExprKind::Literal(_) | ExprKind::Path(_) => {}
    }
}
