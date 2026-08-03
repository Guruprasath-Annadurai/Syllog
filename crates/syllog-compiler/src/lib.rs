//! Syllog compilation orchestration and diagnostic presentation.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::ops::Deref;
use syllog_parser::{Ast, Diagnostic, Severity};
use syllog_semantic::SymbolTable;

/// A front-end compilation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilationPhase {
    /// Source parsing.
    Parse,
    /// Domain configuration validation.
    Validate,
    /// Symbol and type-name resolution.
    Resolve,
    /// Expression and contract type checking.
    TypeCheck,
}

impl std::fmt::Display for CompilationPhase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Parse => "parse",
            Self::Validate => "validate",
            Self::Resolve => "resolve",
            Self::TypeCheck => "type_check",
        })
    }
}

/// A diagnostic attributed to the phase that emitted it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerDiagnostic {
    /// Emitting phase.
    pub phase: CompilationPhase,
    /// Source diagnostic.
    #[serde(flatten)]
    pub diagnostic: Diagnostic,
}

impl Deref for CompilerDiagnostic {
    type Target = Diagnostic;

    fn deref(&self) -> &Self::Target {
        &self.diagnostic
    }
}

/// Complete result of a front-end compilation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compilation {
    /// Logical source filename.
    pub file: String,
    /// Parsed syntax tree when parsing succeeded.
    pub ast: Option<Ast>,
    /// Resolved symbols when semantic analysis ran.
    pub symbols: Option<SymbolTable>,
    /// Attempted phases in order. A failing parse terminates the sequence.
    pub completed_phases: Vec<CompilationPhase>,
    /// All phased diagnostics.
    pub diagnostics: Vec<CompilerDiagnostic>,
}

impl Compilation {
    /// Reports whether the compilation emitted no error diagnostics.
    #[must_use]
    pub fn success(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Runs parsing, configuration validation, resolution, and type checking.
#[must_use]
pub fn compile(file: impl Into<String>, source: &str) -> Compilation {
    let file = file.into();
    let parsed = syllog_parser::check_syl(file.clone(), source);
    let mut diagnostics: Vec<_> = parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| CompilerDiagnostic {
            phase: parser_phase(&diagnostic.code),
            diagnostic,
        })
        .collect();

    let Some(ast) = parsed.ast else {
        return Compilation {
            file,
            ast: None,
            symbols: None,
            completed_phases: vec![CompilationPhase::Parse],
            diagnostics,
        };
    };

    let analysis = syllog_semantic::analyze(&file, &ast);
    diagnostics.extend(
        analysis
            .diagnostics
            .into_iter()
            .map(|diagnostic| CompilerDiagnostic {
                phase: semantic_phase(&diagnostic.code),
                diagnostic,
            }),
    );
    Compilation {
        file,
        ast: Some(ast),
        symbols: Some(analysis.symbols),
        completed_phases: vec![
            CompilationPhase::Parse,
            CompilationPhase::Validate,
            CompilationPhase::Resolve,
            CompilationPhase::TypeCheck,
        ],
        diagnostics,
    }
}

fn parser_phase(code: &str) -> CompilationPhase {
    if code.starts_with("SYL0") {
        CompilationPhase::Parse
    } else {
        CompilationPhase::Validate
    }
}

fn semantic_phase(code: &str) -> CompilationPhase {
    if matches!(code, "SYL2001" | "SYL2002" | "SYL2003" | "SYL2004") {
        CompilationPhase::Resolve
    } else {
        CompilationPhase::TypeCheck
    }
}

/// Renders diagnostics with source lines and primary carets for a terminal.
#[must_use]
pub fn render_human(source: &str, diagnostics: &[CompilerDiagnostic]) -> String {
    let lines: Vec<_> = source.lines().collect();
    let mut rendered = String::new();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        writeln!(
            rendered,
            "{severity}[{}]: {}\n --> {}:{}:{}",
            diagnostic.code,
            diagnostic.message,
            diagnostic.file,
            diagnostic.span.line,
            diagnostic.span.column
        )
        .expect("writing to a String cannot fail");

        if let Some(line) = diagnostic
            .span
            .line
            .checked_sub(1)
            .and_then(|line| lines.get(line))
        {
            let line_number = diagnostic.span.line;
            let gutter_width = line_number.to_string().len();
            writeln!(rendered, "{line_number:>gutter_width$} | {line}")
                .expect("writing to a String cannot fail");
            let indentation = diagnostic.span.column.saturating_sub(1);
            let available = line.chars().count().saturating_sub(indentation);
            let requested = if diagnostic.span.line == diagnostic.span.end_line {
                diagnostic
                    .span
                    .end_column
                    .saturating_sub(diagnostic.span.column)
            } else {
                available
            };
            let caret_count = requested.max(1).min(available.max(1));
            writeln!(
                rendered,
                "{:>gutter_width$} | {}{}",
                "",
                " ".repeat(indentation),
                "^".repeat(caret_count)
            )
            .expect("writing to a String cannot fail");
        }
        writeln!(rendered, " = phase: {}", diagnostic.phase)
            .expect("writing to a String cannot fail");
    }
    rendered
}

/// Zero-based UTF-8 source position for editor integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorPosition {
    /// Zero-based line.
    pub line: usize,
    /// Zero-based UTF-8 column.
    pub column: usize,
    /// Zero-based UTF-8 byte offset.
    pub byte: usize,
}

/// Half-open editor source range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorRange {
    /// Inclusive start.
    pub start: EditorPosition,
    /// Exclusive end.
    pub end: EditorPosition,
}

/// One editor-facing diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorDiagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// Lowercase severity.
    pub severity: String,
    /// Human-readable message without terminal decoration.
    pub message: String,
    /// Logical source filename.
    pub file: String,
    /// Emitting compiler phase.
    pub phase: CompilationPhase,
    /// Zero-based half-open source range.
    pub range: EditorRange,
}

/// Versioned editor-facing compilation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorReport {
    /// Schema version.
    pub schema_version: u32,
    /// Whether compilation succeeded.
    pub success: bool,
    /// Attempted compiler phases.
    pub completed_phases: Vec<CompilationPhase>,
    /// Machine-readable diagnostics only; AST and symbols are intentionally absent.
    pub diagnostics: Vec<EditorDiagnostic>,
}

impl From<&Compilation> for EditorReport {
    fn from(compilation: &Compilation) -> Self {
        Self {
            schema_version: 1,
            success: compilation.success(),
            completed_phases: compilation.completed_phases.clone(),
            diagnostics: compilation
                .diagnostics
                .iter()
                .map(|diagnostic| EditorDiagnostic {
                    code: diagnostic.code.clone(),
                    severity: match diagnostic.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    }
                    .to_owned(),
                    message: diagnostic.message.clone(),
                    file: diagnostic.file.clone(),
                    phase: diagnostic.phase,
                    range: EditorRange {
                        start: EditorPosition {
                            line: diagnostic.span.line.saturating_sub(1),
                            column: diagnostic.span.column.saturating_sub(1),
                            byte: diagnostic.span.start,
                        },
                        end: EditorPosition {
                            line: diagnostic.span.end_line.saturating_sub(1),
                            column: diagnostic.span.end_column.saturating_sub(1),
                            byte: diagnostic.span.end,
                        },
                    },
                })
                .collect(),
        }
    }
}
