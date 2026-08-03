//! Revisioned query database for incremental front-end compilation.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use syllog_parser::{Ast, Diagnostic, Severity};
use syllog_semantic::analyze;

use crate::hir::{
    DefId, HirBlock, HirDefinitionKind, HirExprKind, HirMatchArm, HirModule, HirPattern,
    HirProgram, HirStatement, ModuleId, TypedExpr,
};
use crate::{CompilationPhase, CompilerDiagnostic, lower_to_hir};

/// Stable source identity inside a database session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceFileId(pub u32);

/// Stable package identity inside a database session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PackageId(pub u32);

/// Cached parse and domain-validation result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParseResult {
    /// Parsed tree when syntax is valid.
    pub ast: Option<Ast>,
    /// Parse and domain-validation diagnostics.
    pub diagnostics: Vec<CompilerDiagnostic>,
    /// Query exited cooperatively before producing a cacheable result.
    pub cancelled: bool,
}

/// Cached package HIR result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirResult {
    /// Fully typed HIR when every source succeeds.
    pub program: Option<HirProgram>,
    /// Deterministically ordered package diagnostics.
    pub diagnostics: Vec<CompilerDiagnostic>,
    /// Query exited cooperatively before producing a cacheable result.
    pub cancelled: bool,
}

/// Incremental compiler query interface.
pub trait CompilerDatabase {
    /// Replaces one source input and advances its revision when text changed.
    fn set_source(&mut self, file: SourceFileId, text: Arc<str>);
    /// Parses one source, reusing the exact cached `Arc` when unchanged.
    fn parse(&self, file: SourceFileId) -> Arc<ParseResult>;
    /// Produces typed package HIR from its current source revisions.
    fn hir(&self, package: PackageId) -> Arc<HirResult>;
    /// Returns current package diagnostics.
    fn diagnostics(&self, package: PackageId) -> Arc<[CompilerDiagnostic]>;
}

#[derive(Debug, Clone)]
struct SourceInput {
    text: Arc<str>,
    revision: u64,
}

#[derive(Debug, Clone)]
struct ParseCache {
    revision: u64,
    result: Arc<ParseResult>,
}

#[derive(Debug, Clone)]
struct HirCache {
    fingerprint: Vec<(SourceFileId, u64)>,
    result: Arc<HirResult>,
}

/// Single-owner incremental database. Query caches use interior mutability so
/// read queries retain the ergonomic immutable interface; no shared mutable
/// state is introduced at this milestone.
#[derive(Debug, Default)]
pub struct IncrementalCompilerDatabase {
    sources: BTreeMap<SourceFileId, SourceInput>,
    packages: BTreeMap<PackageId, Vec<SourceFileId>>,
    parse_cache: RefCell<BTreeMap<SourceFileId, ParseCache>>,
    hir_cache: RefCell<BTreeMap<PackageId, HirCache>>,
    cancelled: AtomicBool,
}

impl IncrementalCompilerDatabase {
    /// Creates an empty database.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the deterministic, duplicate-free source membership of a package.
    pub fn set_package_files(&mut self, package: PackageId, mut files: Vec<SourceFileId>) {
        files.sort_unstable();
        files.dedup();
        if self.packages.get(&package) != Some(&files) {
            self.packages.insert(package, files);
            self.hir_cache.get_mut().remove(&package);
        }
    }

    /// Requests cooperative cancellation of subsequent and in-progress query
    /// checkpoints. Cancellation results are never cached.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Clears cancellation before a new top-level compilation request.
    pub fn reset_cancellation(&self) {
        self.cancelled.store(false, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn fingerprint(&self, package: PackageId) -> Option<Vec<(SourceFileId, u64)>> {
        self.packages.get(&package).map(|files| {
            files
                .iter()
                .map(|file| {
                    (
                        *file,
                        self.sources.get(file).map_or(0, |source| source.revision),
                    )
                })
                .collect()
        })
    }

    fn build_hir(&self, package: PackageId) -> HirResult {
        let Some(files) = self.packages.get(&package) else {
            return HirResult {
                program: None,
                diagnostics: vec![database_diagnostic(
                    format!("<package-{}>", package.0),
                    "SYL9002",
                    "unknown package",
                )],
                cancelled: false,
            };
        };
        let mut diagnostics = Vec::new();
        let mut modules = Vec::new();
        let mut entry = None;
        for (module_index, file) in files.iter().enumerate() {
            if self.is_cancelled() {
                return cancelled_hir();
            }
            let parsed = self.parse(*file);
            if parsed.cancelled {
                return cancelled_hir();
            }
            diagnostics.extend(parsed.diagnostics.iter().cloned());
            let Some(ast) = &parsed.ast else {
                continue;
            };
            let filename = source_name(*file);
            let analysis = analyze(&filename, ast);
            diagnostics.extend(analysis.diagnostics.iter().cloned().map(|diagnostic| {
                CompilerDiagnostic {
                    phase: semantic_phase(&diagnostic.code),
                    diagnostic,
                }
            }));
            if !analysis.diagnostics.is_empty() || !parsed.diagnostics.is_empty() {
                continue;
            }
            match lower_to_hir(ast, &analysis.symbols) {
                Ok(program) => {
                    let target = ModuleId(u32::try_from(module_index).unwrap_or(u32::MAX - 1));
                    let mut module = program
                        .modules
                        .into_iter()
                        .next()
                        .expect("HIR has one module");
                    remap_module(&mut module, target);
                    if let Some(candidate) = program.entry.map(|id| remap_id(id, target)) {
                        if entry.replace(candidate).is_some() {
                            diagnostics.push(database_diagnostic(
                                filename,
                                "SYL3002",
                                "package declares more than one main function",
                            ));
                        }
                    }
                    modules.push(module);
                }
                Err(errors) => {
                    diagnostics.extend(errors.into_iter().map(|diagnostic| CompilerDiagnostic {
                        phase: CompilationPhase::TypeCheck,
                        diagnostic,
                    }));
                }
            }
        }
        let success = diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error);
        HirResult {
            program: success.then_some(HirProgram {
                schema_version: 1,
                modules,
                entry,
            }),
            diagnostics,
            cancelled: false,
        }
    }
}

impl CompilerDatabase for IncrementalCompilerDatabase {
    fn set_source(&mut self, file: SourceFileId, text: Arc<str>) {
        let changed = self
            .sources
            .get(&file)
            .is_none_or(|source| source.text != text);
        if !changed {
            return;
        }
        let revision = self
            .sources
            .get(&file)
            .map_or(1, |source| source.revision.saturating_add(1));
        self.sources.insert(file, SourceInput { text, revision });
        self.parse_cache.get_mut().remove(&file);
        let affected = self
            .packages
            .iter()
            .filter(|(_, files)| files.contains(&file))
            .map(|(package, _)| *package)
            .collect::<Vec<_>>();
        let cache = self.hir_cache.get_mut();
        for package in affected {
            cache.remove(&package);
        }
    }

    fn parse(&self, file: SourceFileId) -> Arc<ParseResult> {
        if self.is_cancelled() {
            return Arc::new(ParseResult {
                ast: None,
                diagnostics: Vec::new(),
                cancelled: true,
            });
        }
        let Some(source) = self.sources.get(&file) else {
            return Arc::new(ParseResult {
                ast: None,
                diagnostics: vec![database_diagnostic(
                    source_name(file),
                    "SYL9001",
                    "source input is not set",
                )],
                cancelled: false,
            });
        };
        if let Some(cached) = self.parse_cache.borrow().get(&file)
            && cached.revision == source.revision
        {
            return Arc::clone(&cached.result);
        }
        let checked = syllog_parser::check_syl(source_name(file), &source.text);
        let result = Arc::new(ParseResult {
            ast: checked.ast,
            diagnostics: checked
                .diagnostics
                .into_iter()
                .map(|diagnostic| CompilerDiagnostic {
                    phase: parser_phase(&diagnostic.code),
                    diagnostic,
                })
                .collect(),
            cancelled: false,
        });
        self.parse_cache.borrow_mut().insert(
            file,
            ParseCache {
                revision: source.revision,
                result: Arc::clone(&result),
            },
        );
        result
    }

    fn hir(&self, package: PackageId) -> Arc<HirResult> {
        if self.is_cancelled() {
            return Arc::new(cancelled_hir());
        }
        let Some(fingerprint) = self.fingerprint(package) else {
            return Arc::new(self.build_hir(package));
        };
        if let Some(cached) = self.hir_cache.borrow().get(&package)
            && cached.fingerprint == fingerprint
        {
            return Arc::clone(&cached.result);
        }
        let result = Arc::new(self.build_hir(package));
        if !result.cancelled {
            self.hir_cache.borrow_mut().insert(
                package,
                HirCache {
                    fingerprint,
                    result: Arc::clone(&result),
                },
            );
        }
        result
    }

    fn diagnostics(&self, package: PackageId) -> Arc<[CompilerDiagnostic]> {
        Arc::from(self.hir(package).diagnostics.clone())
    }
}

fn source_name(file: SourceFileId) -> String {
    format!("<source-{}>.syl", file.0)
}

fn cancelled_hir() -> HirResult {
    HirResult {
        program: None,
        diagnostics: Vec::new(),
        cancelled: true,
    }
}

fn database_diagnostic(file: String, code: &str, message: &str) -> CompilerDiagnostic {
    CompilerDiagnostic {
        phase: CompilationPhase::Parse,
        diagnostic: Diagnostic {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            file,
            span: syllog_parser::Span::default(),
        },
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

fn remap_module(module: &mut HirModule, target: ModuleId) {
    module.id = target;
    for definition in &mut module.definitions {
        definition.id = remap_id(definition.id, target);
        match &mut definition.kind {
            HirDefinitionKind::Struct { fields } | HirDefinitionKind::State { fields } => {
                for field in fields {
                    field.id = remap_id(field.id, target);
                }
            }
            HirDefinitionKind::Enum { variants } => {
                for variant in variants {
                    variant.id = remap_id(variant.id, target);
                }
            }
            HirDefinitionKind::Function(function) => {
                for parameter in &mut function.parameters {
                    parameter.id = remap_id(parameter.id, target);
                }
                remap_block(&mut function.body, target);
            }
            HirDefinitionKind::Pipeline(pipeline) => {
                for parameter in &mut pipeline.parameters {
                    parameter.id = remap_id(parameter.id, target);
                }
                pipeline.agent = pipeline.agent.map(|id| remap_id(id, target));
                if let Some(body) = &mut pipeline.body {
                    remap_expression(body, target);
                }
            }
            HirDefinitionKind::Agent | HirDefinitionKind::SafetyBound => {}
        }
    }
}

fn remap_block(block: &mut HirBlock, target: ModuleId) {
    for statement in &mut block.statements {
        match statement {
            HirStatement::Let {
                definition, value, ..
            } => {
                *definition = remap_id(*definition, target);
                remap_expression(value, target);
            }
            HirStatement::Return(value) => {
                if let Some(value) = value {
                    remap_expression(value, target);
                }
            }
            HirStatement::Expression(expression) => remap_expression(expression, target),
        }
    }
}

fn remap_expression(expression: &mut TypedExpr, target: ModuleId) {
    match &mut expression.kind {
        HirExprKind::Reference { definition } => *definition = remap_id(*definition, target),
        HirExprKind::Array(items) => {
            for item in items {
                remap_expression(item, target);
            }
        }
        HirExprKind::Call { callee, arguments } => {
            remap_expression(callee, target);
            for argument in arguments {
                remap_expression(argument, target);
            }
        }
        HirExprKind::Field { base, field } => {
            remap_expression(base, target);
            *field = remap_id(*field, target);
        }
        HirExprKind::Await(operand) | HirExprKind::Unary { operand, .. } => {
            remap_expression(operand, target);
        }
        HirExprKind::Binary { left, right, .. } => {
            remap_expression(left, target);
            remap_expression(right, target);
        }
        HirExprKind::Match { value, arms } => {
            remap_expression(value, target);
            for arm in arms {
                remap_arm(arm, target);
            }
        }
        HirExprKind::Block(block) => remap_block(block, target),
        HirExprKind::Literal(_) => {}
    }
}

fn remap_arm(arm: &mut HirMatchArm, target: ModuleId) {
    remap_pattern(&mut arm.pattern, target);
    if let Some(guard) = &mut arm.guard {
        remap_expression(guard, target);
    }
    remap_expression(&mut arm.body, target);
}

fn remap_pattern(pattern: &mut HirPattern, target: ModuleId) {
    match pattern {
        HirPattern::Binding { definition } => *definition = remap_id(*definition, target),
        HirPattern::Variant { definition, fields } => {
            *definition = remap_id(*definition, target);
            for field in fields {
                remap_pattern(field, target);
            }
        }
        HirPattern::Wildcard | HirPattern::Literal(_) => {}
    }
}

fn remap_id(id: DefId, target: ModuleId) -> DefId {
    if id.module == ModuleId(0) {
        DefId {
            module: target,
            index: id.index,
        }
    } else {
        id
    }
}
