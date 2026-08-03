//! Structured compiler diagnostics.

use crate::{Ast, Span};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// Compilation cannot continue to executable output.
    Error,
    /// Compilation may continue, but attention is recommended.
    Warning,
}

/// One source-positioned compiler diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable diagnostic identifier.
    pub code: String,
    /// Error or warning classification.
    pub severity: Severity,
    /// User-facing explanation.
    pub message: String,
    /// Logical source filename.
    pub file: String,
    /// Primary source range.
    pub span: Span,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}-{}:{}: {:?}[{}]: {}",
            self.file,
            self.span.line,
            self.span.column,
            self.span.end_line,
            self.span.end_column,
            self.severity,
            self.code,
            self.message
        )
    }
}

/// Result of syntax parsing and domain validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    /// Parsed tree; syntax errors prevent its construction.
    pub ast: Option<Ast>,
    /// All diagnostics collected in deterministic validation order.
    pub diagnostics: Vec<Diagnostic>,
}

impl CheckResult {
    /// Reports whether no error diagnostics were emitted.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}
