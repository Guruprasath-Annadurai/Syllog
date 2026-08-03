//! Fully resolved, typed high-level intermediate representation.

use serde::{Deserialize, Serialize};
use syllog_parser::{BinaryOperator, Literal, Span, UnaryOperator};
use syllog_semantic::ResolvedType;

/// Stable module identity within one compilation graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModuleId(pub u32);

/// Stable definition identity within one module revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DefId {
    /// Owning module.
    pub module: ModuleId,
    /// Source-order definition index.
    pub index: u32,
}

/// A versioned typed program suitable for deterministic debug serialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirProgram {
    /// Debug serialization schema.
    pub schema_version: u32,
    /// Modules in stable package order.
    pub modules: Vec<HirModule>,
    /// `main`, when declared.
    pub entry: Option<DefId>,
}

/// One lowered source module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirModule {
    /// Stable module identity.
    pub id: ModuleId,
    /// Top-level definitions in source order.
    pub definitions: Vec<HirDefinition>,
}

/// One named top-level definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirDefinition {
    /// Stable identity used by every reference.
    pub id: DefId,
    /// Source name retained for diagnostics and debug data.
    pub name: String,
    /// Lowered definition payload.
    pub kind: HirDefinitionKind,
    /// Source range.
    pub span: Span,
}

/// Supported HIR definition forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HirDefinitionKind {
    /// Product type.
    Struct {
        /// Product fields.
        fields: Vec<HirMember>,
    },
    /// Tagged union.
    Enum {
        /// Closed variants.
        variants: Vec<HirVariant>,
    },
    /// Executable function.
    Function(HirFunction),
    /// Reactive state declaration.
    State {
        /// State slots.
        fields: Vec<HirMember>,
    },
    /// Model route metadata.
    Agent,
    /// Typed pipeline with an optional result expression.
    Pipeline(HirPipeline),
    /// Safety policy metadata.
    SafetyBound,
}

impl HirDefinitionKind {
    /// Returns the function payload when this is executable function HIR.
    #[must_use]
    pub fn as_function(&self) -> Option<&HirFunction> {
        match self {
            Self::Function(function) => Some(function),
            _ => None,
        }
    }
}

/// A named field with a stable identity and resolved type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirMember {
    /// Field identity.
    pub id: DefId,
    /// Field name.
    pub name: String,
    /// Resolved field type.
    pub ty: ResolvedType,
    /// Source range.
    pub span: Span,
}

/// An enum variant and its resolved payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirVariant {
    /// Variant identity.
    pub id: DefId,
    /// Variant name.
    pub name: String,
    /// Resolved tuple payload types.
    pub fields: Vec<ResolvedType>,
    /// Source range.
    pub span: Span,
}

/// A lowered function signature and body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirFunction {
    /// Whether suspension is permitted.
    pub asynchronous: bool,
    /// Parameters in declaration order.
    pub parameters: Vec<HirParameter>,
    /// Resolved result type.
    pub result: ResolvedType,
    /// Executable body.
    pub body: HirBlock,
}

/// A typed pipeline signature and executable result property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirPipeline {
    /// Parameters in declaration order.
    pub parameters: Vec<HirParameter>,
    /// Resolved output type.
    pub result: ResolvedType,
    /// Selected agent definition.
    pub agent: Option<DefId>,
    /// Result expression when present.
    pub body: Option<TypedExpr>,
}

/// A function or pipeline parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirParameter {
    /// Binding identity.
    pub id: DefId,
    /// Debug name.
    pub name: String,
    /// Resolved type.
    pub ty: ResolvedType,
    /// Source range.
    pub span: Span,
}

/// A sequence of lowered statements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirBlock {
    /// Statements in evaluation order.
    pub statements: Vec<HirStatement>,
    /// Source range.
    pub span: Span,
}

/// A typed executable statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HirStatement {
    /// Local binding.
    Let {
        /// Stable binding identity.
        definition: DefId,
        /// Debug name.
        name: String,
        /// Resolved binding type.
        ty: ResolvedType,
        /// Resolved initializer.
        value: TypedExpr,
    },
    /// Explicit return.
    Return(Option<TypedExpr>),
    /// Expression statement or block result.
    Expression(TypedExpr),
}

/// An expression whose type is fixed before MIR lowering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedExpr {
    /// Resolved expression form.
    pub kind: HirExprKind,
    /// Static type, including error sentinels for unsuccessful builds.
    pub ty: ResolvedType,
    /// Source range.
    pub span: Span,
}

impl TypedExpr {
    /// Visits this expression and all nested executable expressions pre-order.
    pub fn walk(&self, visit: &mut impl FnMut(&TypedExpr)) {
        visit(self);
        match &self.kind {
            HirExprKind::Array(items) => {
                for item in items {
                    item.walk(visit);
                }
            }
            HirExprKind::Call { callee, arguments } => {
                callee.walk(visit);
                for argument in arguments {
                    argument.walk(visit);
                }
            }
            HirExprKind::Field { base, .. } | HirExprKind::Unary { operand: base, .. } => {
                base.walk(visit);
            }
            HirExprKind::Binary { left, right, .. } => {
                left.walk(visit);
                right.walk(visit);
            }
            HirExprKind::Match { value, arms } => {
                value.walk(visit);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        guard.walk(visit);
                    }
                    arm.body.walk(visit);
                }
            }
            HirExprKind::Block(block) => {
                for statement in &block.statements {
                    match statement {
                        HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                            value.walk(visit);
                        }
                        HirStatement::Return(value) => {
                            if let Some(value) = value {
                                value.walk(visit);
                            }
                        }
                    }
                }
            }
            HirExprKind::Literal(_) | HirExprKind::Reference { .. } => {}
        }
    }
}

/// Resolved expression forms. Names that affect execution are represented by
/// identities rather than strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HirExprKind {
    /// Scalar literal.
    Literal(Literal),
    /// Local, global, or variant reference.
    Reference {
        /// Resolved target.
        definition: DefId,
    },
    /// Array elements.
    Array(Vec<TypedExpr>),
    /// Function or constructor invocation.
    Call {
        /// Resolved callable expression.
        callee: Box<TypedExpr>,
        /// Positional argument values.
        arguments: Vec<TypedExpr>,
    },
    /// Resolved product field access.
    Field {
        /// Product value.
        base: Box<TypedExpr>,
        /// Resolved field identity.
        field: DefId,
    },
    /// Prefix operation.
    Unary {
        /// Prefix operator.
        operator: UnaryOperator,
        /// Operand value.
        operand: Box<TypedExpr>,
    },
    /// Infix operation.
    Binary {
        /// Infix operator.
        operator: BinaryOperator,
        /// Left operand.
        left: Box<TypedExpr>,
        /// Right operand.
        right: Box<TypedExpr>,
    },
    /// Closed pattern match.
    Match {
        /// Scrutinized value.
        value: Box<TypedExpr>,
        /// Ordered match arms.
        arms: Vec<HirMatchArm>,
    },
    /// Nested block.
    Block(HirBlock),
}

/// A resolved match arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirMatchArm {
    /// Resolved pattern.
    pub pattern: HirPattern,
    /// Optional Boolean guard.
    pub guard: Option<TypedExpr>,
    /// Typed result.
    pub body: TypedExpr,
    /// Source range.
    pub span: Span,
}

/// A name-resolved match pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HirPattern {
    /// Matches without binding.
    Wildcard,
    /// Introduces a local binding.
    Binding {
        /// Introduced local definition.
        definition: DefId,
    },
    /// Selects an enum or built-in algebraic variant.
    Variant {
        /// Resolved variant identity.
        definition: DefId,
        /// Nested payload patterns.
        fields: Vec<HirPattern>,
    },
    /// Scalar literal.
    Literal(Literal),
}
