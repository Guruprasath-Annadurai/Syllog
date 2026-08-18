//! Deterministic package-wide parsing, resolution, and typed HIR linking.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use syllog_parser::{Ast, Diagnostic, Item, Severity};
use syllog_semantic::{ModuleSource, QualifiedDefId, analyze_modules};

use crate::hir::{DefId, HirModule, HirProgram, ModuleId};
use crate::lower::lower_module_to_hir;
use crate::{CompilationPhase, CompilerDiagnostic};

/// One source input in a package compilation graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSource {
    /// Stable project-relative filename.
    pub file: String,
    /// UTF-8 Syllog source.
    pub source: String,
}

/// Complete result of package-wide front-end compilation and HIR linking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageCompilation {
    /// Linked typed HIR when every source is valid.
    pub hir: Option<HirProgram>,
    /// Deterministically ordered diagnostics from every source.
    pub diagnostics: Vec<CompilerDiagnostic>,
}

impl PackageCompilation {
    /// Reports whether package compilation emitted no error.
    #[must_use]
    pub fn success(&self) -> bool {
        self.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

/// Compiles and links a complete package source graph.
#[must_use]
pub fn compile_package(mut sources: Vec<PackageSource>) -> PackageCompilation {
    sources.sort_by(|left, right| left.file.cmp(&right.file));
    let mut diagnostics = Vec::new();
    if sources.is_empty() {
        diagnostics.push(package_error(
            "<package>",
            "SYL9001",
            "package compilation requires at least one source file",
        ));
        return failed(diagnostics);
    }
    for pair in sources.windows(2) {
        if pair[0].file == pair[1].file {
            diagnostics.push(package_error(
                &pair[1].file,
                "SYL9002",
                "package source filename is duplicated",
            ));
        }
    }
    if has_errors(&diagnostics) {
        return failed(diagnostics);
    }
    let Some(asts) = parse_sources(&sources, &mut diagnostics) else {
        return failed(diagnostics);
    };
    let module_analysis = analyze_modules(
        asts.iter()
            .map(|(file, ast)| ModuleSource {
                file: file.clone(),
                ast: ast.clone(),
            })
            .collect(),
    );
    diagnostics.extend(
        module_analysis
            .diagnostics
            .iter()
            .cloned()
            .map(|diagnostic| CompilerDiagnostic {
                phase: semantic_phase(&diagnostic.code),
                diagnostic,
            }),
    );
    if has_errors(&diagnostics) {
        return failed(diagnostics);
    }
    let mut identities = plan_identities(&module_analysis, &asts);
    let (modules, entry) = link_sources(
        &sources,
        &asts,
        &module_analysis,
        &mut identities,
        &mut diagnostics,
    );
    let hir = HirProgram {
        schema_version: 1,
        modules,
        entry,
    };
    if !has_errors(&diagnostics)
        && let Err(effect_errors) = crate::analyze_effects(&hir)
    {
        diagnostics.extend(effect_errors.into_iter().map(|error| CompilerDiagnostic {
            phase: CompilationPhase::EffectCheck,
            diagnostic: Diagnostic {
                code: error.code.into(),
                severity: Severity::Error,
                message: error.message,
                file: "<package>".into(),
                span: error.span,
            },
        }));
    }
    sort_diagnostics(&mut diagnostics);
    PackageCompilation {
        hir: (!has_errors(&diagnostics)).then_some(hir),
        diagnostics,
    }
}

fn parse_sources(
    sources: &[PackageSource],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Option<BTreeMap<String, Ast>> {
    let mut asts = BTreeMap::new();
    for source in sources {
        let checked = syllog_parser::check_syl(source.file.clone(), &source.source);
        diagnostics.extend(
            checked
                .diagnostics
                .into_iter()
                .map(|diagnostic| CompilerDiagnostic {
                    phase: if diagnostic.code.starts_with("SYL0") {
                        CompilationPhase::Parse
                    } else {
                        CompilationPhase::Validate
                    },
                    diagnostic,
                }),
        );
        if let Some(ast) = checked.ast {
            asts.insert(source.file.clone(), ast);
        }
    }
    (asts.len() == sources.len()).then_some(asts)
}

struct IdentityPlan {
    top_level: BTreeMap<(String, String), DefId>,
    members: BTreeMap<(String, String), DefId>,
    next_by_module: BTreeMap<syllog_semantic::ModuleId, u32>,
}

fn plan_identities(
    analysis: &syllog_semantic::ModuleAnalysis,
    asts: &BTreeMap<String, Ast>,
) -> IdentityPlan {
    let mut top_level = BTreeMap::new();
    let mut members = BTreeMap::new();
    let mut next_by_module = BTreeMap::new();
    for module in &analysis.modules {
        let module_id = ModuleId(module.id.0);
        for definition in module.definitions.values() {
            top_level.insert(
                (module.name.clone(), definition.name.clone()),
                DefId {
                    module: module_id,
                    index: definition.id.index,
                },
            );
        }
        let mut next = u32::try_from(module.definitions.len()).unwrap_or(u32::MAX);
        for file in &module.files {
            if let Some(ast) = asts.get(file) {
                for item in &ast.items {
                    for key in member_keys(item) {
                        members.insert(
                            (module.name.clone(), key),
                            DefId {
                                module: module_id,
                                index: next,
                            },
                        );
                        next = next.saturating_add(1);
                    }
                }
            }
        }
        next_by_module.insert(module.id, next);
    }
    IdentityPlan {
        top_level,
        members,
        next_by_module,
    }
}

fn link_sources(
    sources: &[PackageSource],
    asts: &BTreeMap<String, Ast>,
    analysis: &syllog_semantic::ModuleAnalysis,
    identities: &mut IdentityPlan,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> (Vec<HirModule>, Option<DefId>) {
    let mut linked_modules = BTreeMap::<ModuleId, HirModule>::new();
    let mut entry = None;
    for source in sources {
        let ast = &asts[&source.file];
        let Some(module_name) = ast
            .module
            .as_ref()
            .map(|declaration| declaration.path.join("::"))
        else {
            continue;
        };
        let Some(module) = analysis.module(&module_name) else {
            diagnostics.push(package_error(
                &source.file,
                "SYL9003",
                "analyzed module disappeared before HIR linking",
            ));
            continue;
        };
        let module_id = ModuleId(module.id.0);
        let (globals, planned_members) = planned_names(
            module,
            &module_name,
            asts,
            analysis,
            identities,
            &source.file,
            diagnostics,
        );
        let Some(source_analysis) = analysis.source_analyses.get(&source.file) else {
            diagnostics.push(package_error(
                &source.file,
                "SYL9003",
                "source analysis disappeared before HIR linking",
            ));
            continue;
        };
        let next = identities
            .next_by_module
            .get(&module.id)
            .copied()
            .unwrap_or(0);
        match lower_module_to_hir(
            ast,
            source_analysis,
            module_id,
            globals,
            planned_members,
            next,
        ) {
            Ok((lowered, candidate, next)) => {
                identities.next_by_module.insert(module.id, next);
                linked_modules
                    .entry(module_id)
                    .or_insert_with(|| HirModule {
                        id: module_id,
                        definitions: Vec::new(),
                    })
                    .definitions
                    .extend(lowered.definitions);
                if let Some(candidate) = candidate
                    && entry.replace(candidate).is_some()
                {
                    diagnostics.push(package_error(
                        &source.file,
                        "SYL3002",
                        "package declares more than one main function",
                    ));
                }
            }
            Err(errors) => {
                diagnostics.extend(errors.into_iter().map(|diagnostic| CompilerDiagnostic {
                    phase: CompilationPhase::TypeCheck,
                    diagnostic,
                }));
            }
        }
    }
    (linked_modules.into_values().collect(), entry)
}

fn planned_names(
    module: &syllog_semantic::ResolvedModule,
    module_name: &str,
    asts: &BTreeMap<String, Ast>,
    analysis: &syllog_semantic::ModuleAnalysis,
    identities: &IdentityPlan,
    file: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> (BTreeMap<String, DefId>, BTreeMap<String, DefId>) {
    let mut globals = module
        .definitions
        .keys()
        .filter_map(|name| {
            identities
                .top_level
                .get(&(module_name.to_owned(), name.clone()))
                .map(|id| (name.clone(), *id))
        })
        .collect::<BTreeMap<_, _>>();
    let mut planned_members = identities
        .members
        .iter()
        .filter(|((owner, _), _)| owner == module_name)
        .map(|((_, key), id)| (key.clone(), *id))
        .collect::<BTreeMap<_, _>>();
    for import in module.imports.values() {
        let Some(target) = find_definition(analysis, import.definition) else {
            diagnostics.push(package_error(
                file,
                "SYL9003",
                "resolved import target disappeared before HIR linking",
            ));
            continue;
        };
        globals.insert(
            import.local_name.clone(),
            DefId {
                module: ModuleId(import.definition.module.0),
                index: import.definition.index,
            },
        );
        let Some(target_module) = analysis
            .modules
            .iter()
            .find(|candidate| candidate.id == import.definition.module)
        else {
            continue;
        };
        let Some(item) = asts
            .get(&target.file)
            .and_then(|ast| ast.items.iter().find(|item| item_name(item) == target.name))
        else {
            continue;
        };
        for key in member_keys(item) {
            if let Some(id) = identities
                .members
                .get(&(target_module.name.clone(), key.clone()))
            {
                planned_members.insert(
                    rename_member_owner(&key, &target.name, &import.local_name),
                    *id,
                );
            }
        }
    }
    (globals, planned_members)
}

fn failed(mut diagnostics: Vec<CompilerDiagnostic>) -> PackageCompilation {
    sort_diagnostics(&mut diagnostics);
    PackageCompilation {
        hir: None,
        diagnostics,
    }
}

fn has_errors(diagnostics: &[CompilerDiagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
}

fn find_definition(
    analysis: &syllog_semantic::ModuleAnalysis,
    id: QualifiedDefId,
) -> Option<&syllog_semantic::ModuleDefinition> {
    analysis
        .modules
        .iter()
        .find(|module| module.id == id.module)?
        .definitions
        .values()
        .find(|definition| definition.id == id)
}

fn member_keys(item: &Item) -> Vec<String> {
    match item {
        Item::Struct(node) => node
            .fields
            .iter()
            .map(|field| format!("{}.{}", node.name, field.name))
            .collect(),
        Item::Enum(node) => node
            .variants
            .iter()
            .map(|variant| format!("{}::{}", node.name, variant.name))
            .collect(),
        Item::State(node) => node
            .fields
            .iter()
            .map(|field| format!("{}.{}", node.name, field.name))
            .collect(),
        _ => Vec::new(),
    }
}

fn rename_member_owner(key: &str, owner: &str, alias: &str) -> String {
    key.strip_prefix(owner)
        .map_or_else(|| key.to_owned(), |suffix| format!("{alias}{suffix}"))
}

fn item_name(item: &Item) -> &str {
    match item {
        Item::Struct(node) => &node.name,
        Item::Enum(node) => &node.name,
        Item::Function(node) => &node.name,
        Item::State(node) => &node.name,
        Item::Agent(node) => &node.name,
        Item::Pipeline(node) => &node.name,
        Item::SafetyBound(node) => &node.name,
    }
}

fn semantic_phase(code: &str) -> CompilationPhase {
    if matches!(
        code,
        "SYL2001"
            | "SYL2002"
            | "SYL2003"
            | "SYL2004"
            | "SYL2400"
            | "SYL2401"
            | "SYL2402"
            | "SYL2403"
            | "SYL2404"
    ) {
        CompilationPhase::Resolve
    } else {
        CompilationPhase::TypeCheck
    }
}

fn sort_diagnostics(diagnostics: &mut [CompilerDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        (&left.file, left.span.start, &left.code).cmp(&(&right.file, right.span.start, &right.code))
    });
}

fn package_error(file: &str, code: &str, message: &str) -> CompilerDiagnostic {
    CompilerDiagnostic {
        phase: CompilationPhase::Resolve,
        diagnostic: Diagnostic {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            file: file.into(),
            span: syllog_parser::Span::default(),
        },
    }
}
