//! Span-aware typed syntax tree for the Syllog front end.

use serde::{Deserialize, Serialize};

/// A half-open UTF-8 byte range and its one-based source coordinates.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Span {
    /// Inclusive byte offset.
    pub start: usize,
    /// Exclusive byte offset.
    pub end: usize,
    /// One-based starting line.
    pub line: usize,
    /// One-based starting column.
    pub column: usize,
    /// One-based ending line.
    pub end_line: usize,
    /// One-based ending column.
    pub end_column: usize,
}

/// A complete Syllog source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ast {
    /// Declared logical module, when this is a project source file.
    pub module: Option<ModuleNode>,
    /// Imports declared before top-level items.
    pub imports: Vec<UseNode>,
    /// Top-level declarations in source order.
    pub items: Vec<Item>,
    /// Span covering the complete compilation unit.
    pub span: Span,
}

/// Logical module declaration for a project source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleNode {
    /// Qualified module path segments.
    pub path: Vec<String>,
    /// Full declaration range.
    pub span: Span,
}

/// One imported definition with an optional local alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseNode {
    /// Qualified path ending in the imported definition name.
    pub path: Vec<String>,
    /// Explicit local alias, when present.
    pub alias: Option<String>,
    /// Full import range.
    pub span: Span,
}

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Item {
    /// A product type declaration.
    Struct(StructNode),
    /// A tagged-union declaration.
    Enum(EnumNode),
    /// A function declaration.
    Function(FunctionNode),
    /// A reactive state declaration.
    State(StateNode),
    /// A first-class model route.
    Agent(AgentNode),
    /// A typed orchestration declaration.
    Pipeline(PipelineNode),
    /// A mandatory policy gate.
    SafetyBound(SafetyBoundNode),
}

/// A named type expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeNode {
    /// Type syntax.
    pub kind: TypeKind,
    /// Source range of this type.
    pub span: Span,
}

/// Supported type forms in the current front end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeKind {
    /// A shared or exclusive reference with an optional named lifetime.
    Reference {
        /// Explicit lifetime such as `'request`, without the leading quote.
        lifetime: Option<String>,
        /// Whether this is an exclusive mutable borrow.
        mutable: bool,
        /// Referenced type.
        inner: Box<TypeNode>,
    },
    /// A qualified path with optional generic arguments.
    Path {
        /// Qualified path segments.
        segments: Vec<String>,
        /// Generic type arguments.
        arguments: Vec<TypeNode>,
    },
    /// A dynamically sized array/slice type.
    Array(Box<TypeNode>),
    /// A tuple type.
    Tuple(Vec<TypeNode>),
}

/// A field in a struct declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructField {
    /// Whether the field is public.
    pub public: bool,
    /// Field name.
    pub name: String,
    /// Declared field type.
    pub ty: TypeNode,
    /// Source range of the field.
    pub span: Span,
}

/// A product type declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructNode {
    /// Whether the declaration is public.
    pub public: bool,
    /// Type name.
    pub name: String,
    /// Ordered fields.
    pub fields: Vec<StructField>,
    /// Full declaration range.
    pub span: Span,
}

/// One enum variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumVariant {
    /// Variant name.
    pub name: String,
    /// Tuple payload types.
    pub fields: Vec<TypeNode>,
    /// Full variant range.
    pub span: Span,
}

/// A tagged-union declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumNode {
    /// Whether the declaration is public.
    pub public: bool,
    /// Type name.
    pub name: String,
    /// Declared variants.
    pub variants: Vec<EnumVariant>,
    /// Full declaration range.
    pub span: Span,
}

/// A function parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Parameter name.
    pub name: String,
    /// Parameter type.
    pub ty: TypeNode,
    /// Full parameter range.
    pub span: Span,
}

/// A statement block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Statements in evaluation order.
    pub statements: Vec<Statement>,
    /// Braced source range.
    pub span: Span,
}

/// A statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Statement {
    /// Statement syntax.
    pub kind: StatementKind,
    /// Full statement range.
    pub span: Span,
}

/// Supported statement forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StatementKind {
    /// A local binding.
    Let {
        /// Binding name.
        name: String,
        /// Optional explicit type.
        ty: Option<TypeNode>,
        /// Initial value.
        value: Expr,
    },
    /// An explicit return.
    Return(Option<Expr>),
    /// An expression evaluated for its result or effects.
    Expression(Expr),
}

/// A function declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionNode {
    /// Declarative compiler attributes in source order.
    pub attributes: Vec<AttributeNode>,
    /// Whether the declaration is public.
    pub public: bool,
    /// Whether the function may suspend.
    pub asynchronous: bool,
    /// Function name.
    pub name: String,
    /// Typed parameters.
    pub parameters: Vec<Parameter>,
    /// Optional result type; absence means `()`.
    pub return_type: Option<TypeNode>,
    /// Explicit upper bound on effects; an absent set is inferred for private functions.
    pub effects: Option<Vec<EffectNode>>,
    /// Function body.
    pub body: Block,
    /// Full declaration range.
    pub span: Span,
}

/// One source-spanned function effect declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectNode {
    /// Canonical source spelling.
    pub name: String,
    /// Exact effect-name range.
    pub span: Span,
}

/// Compiler metadata attached to a declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeNode {
    /// Attribute name without delimiters.
    pub name: String,
    /// Full `#[name]` source range.
    pub span: Span,
}

/// A field owned by a reactive state declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateField {
    /// State slot name.
    pub name: String,
    /// Declared slot type.
    pub ty: TypeNode,
    /// Optional initial value.
    pub initializer: Option<Expr>,
    /// Full field range.
    pub span: Span,
}

/// A reactive state declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateNode {
    /// Whether the declaration is public.
    pub public: bool,
    /// State type name.
    pub name: String,
    /// Owned state slots.
    pub fields: Vec<StateField>,
    /// Full declaration range.
    pub span: Span,
}

/// A typed property in an agent, pipeline, or safety-bound declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    /// Property name.
    pub name: String,
    /// Optional explicit type annotation.
    pub ty: Option<TypeNode>,
    /// Property value expression.
    pub value: Expr,
    /// Full property range.
    pub span: Span,
}

/// Backwards-compatible name for a domain property.
pub type Field = Property;

/// Literal values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    /// A quoted UTF-8 string.
    String(String),
    /// A symbolic identifier used as a scalar value.
    Identifier(String),
    /// A signed 64-bit integer.
    Integer(i64),
    /// An IEEE-754 floating-point literal, retained textually for exact parsing.
    Float(String),
    /// A Boolean value.
    Boolean(bool),
}

/// An expression with an exact source range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expr {
    /// Expression syntax.
    pub kind: ExprKind,
    /// Full expression range.
    pub span: Span,
}

/// Supported expression forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExprKind {
    /// Creates a shared or exclusive lexical borrow.
    Borrow {
        /// Whether the borrow is exclusive and mutable.
        mutable: bool,
        /// Borrowed place expression.
        operand: Box<Expr>,
    },
    /// Suspends the enclosing async function until the operand is ready.
    Await(Box<Expr>),
    /// A scalar literal.
    Literal(Literal),
    /// A qualified name.
    Path(Vec<String>),
    /// An array literal.
    Array(Vec<Expr>),
    /// A function or constructor call.
    Call {
        /// Called expression.
        callee: Box<Expr>,
        /// Positional or named arguments.
        arguments: Vec<CallArgument>,
    },
    /// Field access.
    Field {
        /// Base expression.
        base: Box<Expr>,
        /// Selected field name.
        name: String,
    },
    /// Prefix operator application.
    Unary {
        /// Operator.
        operator: UnaryOperator,
        /// Operand.
        operand: Box<Expr>,
    },
    /// Infix operator application.
    Binary {
        /// Operator.
        operator: BinaryOperator,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
    },
    /// Exhaustive pattern matching.
    Match {
        /// Scrutinized expression.
        value: Box<Expr>,
        /// Ordered match arms.
        arms: Vec<MatchArm>,
    },
    /// A braced expression block.
    Block(Block),
}

/// One call argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallArgument {
    /// Optional named-argument label.
    pub name: Option<String>,
    /// Argument expression.
    pub value: Expr,
    /// Full argument range.
    pub span: Span,
}

/// A pattern with an exact source range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    /// Pattern syntax.
    pub kind: PatternKind,
    /// Full pattern range.
    pub span: Span,
}

/// Supported pattern forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternKind {
    /// Matches any value without binding it.
    Wildcard,
    /// Binds or selects a qualified name.
    Path(Vec<String>),
    /// A constructor pattern with nested payload patterns.
    Constructor {
        /// Constructor path.
        path: Vec<String>,
        /// Payload patterns.
        fields: Vec<Pattern>,
    },
    /// A scalar literal pattern.
    Literal(Literal),
}

/// One `match` arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    /// Arm pattern.
    pub pattern: Pattern,
    /// Optional Boolean guard.
    pub guard: Option<Expr>,
    /// Arm result.
    pub body: Expr,
    /// Full arm range.
    pub span: Span,
}

/// Prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOperator {
    /// Numeric negation.
    Negate,
    /// Boolean negation.
    Not,
}

/// Infix operators, ordered independently by the parser's precedence table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOperator {
    /// Addition.
    Add,
    /// Subtraction.
    Subtract,
    /// Multiplication.
    Multiply,
    /// Division.
    Divide,
    /// Remainder.
    Remainder,
    /// Equality.
    Equal,
    /// Inequality.
    NotEqual,
    /// Less-than comparison.
    Less,
    /// Less-than-or-equal comparison.
    LessEqual,
    /// Greater-than comparison.
    Greater,
    /// Greater-than-or-equal comparison.
    GreaterEqual,
    /// Short-circuit Boolean conjunction.
    And,
    /// Short-circuit Boolean disjunction.
    Or,
}

macro_rules! domain_node {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            /// Whether the declaration is public.
            pub public: bool,
            /// Declaration name.
            pub name: String,
            /// Typed declaration properties.
            pub fields: Vec<Property>,
            /// Full declaration range.
            pub span: Span,
        }

        impl $name {
            /// Returns the last scalar literal assigned to `name`.
            #[must_use]
            pub fn field(&self, name: &str) -> Option<&Literal> {
                self.fields
                    .iter()
                    .rev()
                    .find(|field| field.name == name)
                    .and_then(|field| match &field.value.kind {
                        ExprKind::Literal(literal) => Some(literal),
                        _ => None,
                    })
            }

            /// Returns the complete typed property assigned to `name`.
            #[must_use]
            pub fn property(&self, name: &str) -> Option<&Property> {
                self.fields.iter().rev().find(|field| field.name == name)
            }
        }
    };
}

domain_node!(AgentNode, "A first-class model route declaration.");

/// A typed streaming pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineNode {
    /// Whether the declaration is public.
    pub public: bool,
    /// Pipeline name.
    pub name: String,
    /// Typed pipeline inputs.
    pub parameters: Vec<Parameter>,
    /// Optional pipeline result type.
    pub return_type: Option<TypeNode>,
    /// Typed configuration and body properties.
    pub fields: Vec<Property>,
    /// Full declaration range.
    pub span: Span,
}

impl PipelineNode {
    /// Returns the last scalar literal assigned to `name`.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Literal> {
        literal_property(&self.fields, name)
    }

    /// Returns the complete typed property assigned to `name`.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.fields.iter().rev().find(|field| field.name == name)
    }
}

/// A typed safety gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyBoundNode {
    /// Whether the declaration is public.
    pub public: bool,
    /// Bound name.
    pub name: String,
    /// Typed values inspected by the bound.
    pub parameters: Vec<Parameter>,
    /// Typed requirements and configuration.
    pub fields: Vec<Property>,
    /// Full declaration range.
    pub span: Span,
}

impl SafetyBoundNode {
    /// Returns the last scalar literal assigned to `name`.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Literal> {
        literal_property(&self.fields, name)
    }

    /// Returns the complete typed property assigned to `name`.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.fields.iter().rev().find(|field| field.name == name)
    }
}

fn literal_property<'a>(fields: &'a [Property], name: &str) -> Option<&'a Literal> {
    fields
        .iter()
        .rev()
        .find(|field| field.name == name)
        .and_then(|field| match &field.value.kind {
            ExprKind::Literal(literal) => Some(literal),
            _ => None,
        })
}
