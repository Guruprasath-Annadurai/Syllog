//! Resolved types and public symbol metadata.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use syllog_parser::Span;

/// Primitive types built into every Syllog module.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PrimitiveType {
    /// Boolean.
    Bool,
    /// Owned UTF-8 string.
    String,
    /// Borrowed UTF-8 string.
    Str,
    /// Unicode scalar.
    Char,
    /// Owned bytes.
    Bytes,
    /// Signed integer with the displayed width or target width.
    Signed(String),
    /// Unsigned integer with the displayed width or target width.
    Unsigned(String),
    /// Floating point with the displayed width.
    Float(String),
    /// Runtime duration.
    Duration,
    /// Runtime byte size.
    Size,
    /// Domain-level agent reference.
    AgentRef,
    /// Domain-level provider definition.
    Provider,
    /// Text alias used by compact agent declarations.
    Text,
}

/// A type after name and generic-argument resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedType {
    /// Built-in primitive.
    Primitive(PrimitiveType),
    /// User product type.
    Struct(String),
    /// User tagged union.
    Enum(String),
    /// Reactive state type.
    State(String),
    /// Optional value.
    Option(Box<ResolvedType>),
    /// Success or error value.
    Result(Box<ResolvedType>, Box<ResolvedType>),
    /// Dynamic array.
    Array(Box<ResolvedType>),
    /// Tuple.
    Tuple(Vec<ResolvedType>),
    /// Function signature.
    Function(Vec<ResolvedType>, Box<ResolvedType>),
    /// Unit value.
    Unit,
    /// Unconstrained empty collection or expression.
    Unknown,
    /// Error sentinel used to suppress cascading diagnostics.
    Error,
    /// Unsuffixed integer literal.
    IntegerLiteral,
    /// Unsuffixed floating literal.
    FloatLiteral,
}

/// The resolved type assigned to one source expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpressionType {
    /// Expression source range.
    pub span: Span,
    /// Type after inference and compatibility checking.
    pub ty: ResolvedType,
}

impl std::fmt::Display for ResolvedType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Primitive(primitive) => write!(formatter, "{primitive:?}"),
            Self::Struct(name) | Self::Enum(name) | Self::State(name) => formatter.write_str(name),
            Self::Option(inner) => write!(formatter, "Option<{inner}>"),
            Self::Result(ok, error) => write!(formatter, "Result<{ok}, {error}>"),
            Self::Array(inner) => write!(formatter, "[{inner}]"),
            Self::Tuple(items) => {
                formatter.write_str("(")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{item}")?;
                }
                formatter.write_str(")")
            }
            Self::Function(parameters, result) => {
                write!(formatter, "fn({parameters:?}) -> {result}")
            }
            Self::Unit => formatter.write_str("()"),
            Self::Unknown => formatter.write_str("<unknown>"),
            Self::Error => formatter.write_str("<error>"),
            Self::IntegerLiteral => formatter.write_str("<integer>"),
            Self::FloatLiteral => formatter.write_str("<float>"),
        }
    }
}

/// Kind of a global type symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeSymbolKind {
    /// Primitive type.
    Primitive,
    /// Product type.
    Struct,
    /// Tagged union.
    Enum,
    /// Reactive state type.
    State,
    /// Built-in generic constructor.
    Generic {
        /// Required generic argument count.
        arity: usize,
    },
}

/// One global type symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeSymbol {
    /// Symbol name.
    pub name: String,
    /// Symbol category.
    pub kind: TypeSymbolKind,
    /// Declaring range, or an empty range for built-ins.
    pub span: Span,
}

/// Kind of a global value symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueSymbolKind {
    /// Function.
    Function,
    /// Agent configuration.
    Agent,
    /// Pipeline.
    Pipeline,
    /// Safety bound.
    SafetyBound,
}

/// One global value symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueSymbol {
    /// Symbol name.
    pub name: String,
    /// Symbol category.
    pub kind: ValueSymbolKind,
    /// Declaring range.
    pub span: Span,
}

/// Separate global type and value namespaces.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolTable {
    /// Global type namespace.
    pub types: BTreeMap<String, TypeSymbol>,
    /// Global value namespace.
    pub values: BTreeMap<String, ValueSymbol>,
}

impl SymbolTable {
    /// Reports whether `name` is present in the type namespace.
    #[must_use]
    pub fn contains_type(&self, name: &str) -> bool {
        self.types.contains_key(name)
    }

    /// Reports whether `name` is present in the value namespace.
    #[must_use]
    pub fn contains_value(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }
}
