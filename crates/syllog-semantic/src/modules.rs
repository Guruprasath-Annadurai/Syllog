//! Deterministic cross-file module graph and visibility analysis.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use syllog_parser::{Ast, Diagnostic, Item, Severity, Span, TypeKind, TypeNode, UseNode};

use super::{Analysis, analyze_with_imports};

/// One parsed source file participating in module analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleSource {
    /// Stable logical filename used in diagnostics and ordering.
    pub file: String,
    /// Parsed syntax tree.
    pub ast: Ast,
}

/// Stable module identity assigned by qualified module-name ordering.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ModuleId(pub u32);

/// Definition identity qualified by its owning module.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct QualifiedDefId {
    /// Owning module.
    pub module: ModuleId,
    /// Module-local stable definition index.
    pub index: u32,
}

/// Coarse declaration category retained by the graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleDefinitionKind {
    /// Product type.
    Struct,
    /// Tagged union.
    Enum,
    /// Function.
    Function,
    /// Reactive state.
    State,
    /// Agent route.
    Agent,
    /// Pipeline.
    Pipeline,
    /// Safety bound.
    SafetyBound,
}

/// One module-owned declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleDefinition {
    /// Qualified identity.
    pub id: QualifiedDefId,
    /// Source name.
    pub name: String,
    /// Declaration category.
    pub kind: ModuleDefinitionKind,
    /// Whether other modules may import it.
    pub public: bool,
    /// Declaring filename.
    pub file: String,
    /// Declaring source range.
    pub span: Span,
}

/// One successfully resolved local import.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedImport {
    /// Local name, including an explicit alias when present.
    pub local_name: String,
    /// Imported qualified definition.
    pub definition: QualifiedDefId,
    /// Import statement source range.
    pub span: Span,
}

/// A resolved logical module assembled from one or more source files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedModule {
    /// Stable identity.
    pub id: ModuleId,
    /// Qualified logical name.
    pub name: String,
    /// Source files in deterministic order.
    pub files: Vec<String>,
    /// Definitions indexed by source name.
    pub definitions: BTreeMap<String, ModuleDefinition>,
    /// Successful imports indexed by local name.
    pub imports: BTreeMap<String, ResolvedImport>,
    /// Direct module dependencies.
    pub dependencies: BTreeSet<ModuleId>,
    /// Hash of exported signatures only, excluding private implementation bodies.
    pub interface_hash: [u8; 32],
}

/// Complete deterministic module analysis.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModuleAnalysis {
    /// Modules ordered by qualified name.
    pub modules: Vec<ResolvedModule>,
    /// Per-file body analysis after imported symbols are installed.
    pub source_analyses: BTreeMap<String, Analysis>,
    /// Stable source-positioned graph diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl ModuleAnalysis {
    /// Finds a resolved module by qualified name.
    #[must_use]
    pub fn module(&self, name: &str) -> Option<&ResolvedModule> {
        self.modules.iter().find(|module| module.name == name)
    }
}

#[derive(Clone)]
struct PendingImport {
    owner: ModuleId,
    file: String,
    node: UseNode,
}

/// Constructs and validates a deterministic module graph before body analysis.
#[must_use]
pub fn analyze_modules(mut sources: Vec<ModuleSource>) -> ModuleAnalysis {
    sources.sort_by(|left, right| left.file.cmp(&right.file));
    let mut diagnostics = Vec::new();
    let names = sources
        .iter()
        .filter_map(|source| source.ast.module.as_ref())
        .map(|module| module.path.join("::"))
        .collect::<BTreeSet<_>>();
    let ids = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, ModuleId(u32::try_from(index).unwrap_or(u32::MAX))))
        .collect::<BTreeMap<_, _>>();
    let mut modules = ids
        .iter()
        .map(|(name, id)| {
            (
                *id,
                ResolvedModule {
                    id: *id,
                    name: name.clone(),
                    files: Vec::new(),
                    definitions: BTreeMap::new(),
                    imports: BTreeMap::new(),
                    dependencies: BTreeSet::new(),
                    interface_hash: [0; 32],
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut pending = Vec::new();
    collect_sources(&sources, &ids, &mut modules, &mut pending, &mut diagnostics);
    let dependency_sites = resolve_imports(&ids, &mut modules, pending, &mut diagnostics);
    if let Some(cycle) = find_cycle(&modules) {
        if let Some(first) = cycle.first().and_then(|id| modules.get(id)) {
            let site = cycle
                .windows(2)
                .next()
                .and_then(|edge| dependency_sites.get(&(edge[0], edge[1])))
                .cloned()
                .unwrap_or_else(|| {
                    (
                        first.files.first().cloned().unwrap_or_default(),
                        Span::default(),
                    )
                });
            diagnostics.push(error(
                "SYL2404",
                format!(
                    "module dependency cycle: {}",
                    cycle
                        .iter()
                        .filter_map(|id| modules.get(id).map(|module| module.name.as_str()))
                        .collect::<Vec<_>>()
                        .join(" -> ")
                ),
                site.0,
                site.1,
            ));
        }
    }
    for module in modules.values_mut() {
        module.interface_hash = interface_hash(module, &sources);
    }
    let source_analyses = analyze_source_bodies(&sources, &modules);
    diagnostics.extend(
        source_analyses
            .values()
            .flat_map(|analysis| analysis.diagnostics.iter().cloned()),
    );
    diagnostics.sort_by(|left, right| {
        (&left.file, left.span.start, &left.code).cmp(&(&right.file, right.span.start, &right.code))
    });
    ModuleAnalysis {
        modules: modules.into_values().collect(),
        source_analyses,
        diagnostics,
    }
}

fn analyze_source_bodies(
    sources: &[ModuleSource],
    modules: &BTreeMap<ModuleId, ResolvedModule>,
) -> BTreeMap<String, Analysis> {
    let items = modules
        .values()
        .flat_map(|module| module.definitions.values())
        .filter_map(|definition| {
            let source = sources
                .iter()
                .find(|source| source.file == definition.file)?;
            let item =
                source.ast.items.iter().find(|item| {
                    item_identity(item) == (definition.name.as_str(), definition.span)
                })?;
            Some((definition.id, item))
        })
        .collect::<BTreeMap<_, _>>();
    let mut analyses = BTreeMap::new();
    for source in sources {
        let Some(module_name) = source
            .ast
            .module
            .as_ref()
            .map(|declaration| declaration.path.join("::"))
        else {
            continue;
        };
        let Some(module) = modules.values().find(|module| module.name == module_name) else {
            continue;
        };
        let imports = module
            .imports
            .values()
            .filter_map(|import| {
                items
                    .get(&import.definition)
                    .map(|item| (import.local_name.as_str(), *item))
            })
            .chain(module.definitions.values().filter_map(|definition| {
                if definition.file == source.file
                    || source
                        .ast
                        .items
                        .iter()
                        .any(|item| item_identity(item).0 == definition.name)
                {
                    return None;
                }
                items
                    .get(&definition.id)
                    .map(|item| (definition.name.as_str(), *item))
            }))
            .collect::<Vec<_>>();
        analyses.insert(
            source.file.clone(),
            analyze_with_imports(&source.file, &source.ast, &imports),
        );
    }
    analyses
}

fn collect_sources(
    sources: &[ModuleSource],
    ids: &BTreeMap<String, ModuleId>,
    modules: &mut BTreeMap<ModuleId, ResolvedModule>,
    pending: &mut Vec<PendingImport>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut next_indices = BTreeMap::<ModuleId, u32>::new();
    for source in sources {
        let Some(declaration) = &source.ast.module else {
            diagnostics.push(error(
                "SYL2400",
                "project source must declare a module".into(),
                source.file.clone(),
                source.ast.span,
            ));
            continue;
        };
        let name = declaration.path.join("::");
        let Some(id) = ids.get(&name).copied() else {
            continue;
        };
        let Some(module) = modules.get_mut(&id) else {
            continue;
        };
        module.files.push(source.file.clone());
        for item in &source.ast.items {
            let index = next_indices.entry(id).or_default();
            let definition = module_definition(id, *index, &source.file, item);
            *index = index.saturating_add(1);
            if module.definitions.contains_key(&definition.name) {
                diagnostics.push(error(
                    "SYL2403",
                    format!(
                        "duplicate definition '{}' exported by module '{name}'",
                        definition.name
                    ),
                    source.file.clone(),
                    definition.span,
                ));
            } else {
                module
                    .definitions
                    .insert(definition.name.clone(), definition);
            }
        }
        pending.extend(
            source
                .ast
                .imports
                .iter()
                .cloned()
                .map(|node| PendingImport {
                    owner: id,
                    file: source.file.clone(),
                    node,
                }),
        );
    }
}

fn resolve_imports(
    ids: &BTreeMap<String, ModuleId>,
    modules: &mut BTreeMap<ModuleId, ResolvedModule>,
    pending: Vec<PendingImport>,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<(ModuleId, ModuleId), (String, Span)> {
    let mut dependency_sites = BTreeMap::new();
    for import in pending {
        let Some((symbol, module_path)) = import.node.path.split_last() else {
            continue;
        };
        let module_name = module_path.join("::");
        let Some(target_id) = ids.get(&module_name).copied() else {
            diagnostics.push(error(
                "SYL2401",
                format!("unknown imported module '{module_name}'"),
                import.file,
                import.node.span,
            ));
            continue;
        };
        let definition = modules
            .get(&target_id)
            .and_then(|module| module.definitions.get(symbol))
            .cloned();
        let Some(definition) = definition else {
            diagnostics.push(error(
                "SYL2401",
                format!("module '{module_name}' has no definition '{symbol}'"),
                import.file,
                import.node.span,
            ));
            continue;
        };
        if !definition.public {
            diagnostics.push(error(
                "SYL2402",
                format!("definition '{module_name}::{symbol}' is private"),
                import.file,
                import.node.span,
            ));
            continue;
        }
        let local_name = import.node.alias.clone().unwrap_or_else(|| symbol.clone());
        let Some(owner) = modules.get_mut(&import.owner) else {
            continue;
        };
        owner.dependencies.insert(target_id);
        dependency_sites
            .entry((import.owner, target_id))
            .or_insert_with(|| (import.file.clone(), import.node.span));
        if owner.definitions.contains_key(&local_name) || owner.imports.contains_key(&local_name) {
            diagnostics.push(error(
                "SYL2403",
                format!("duplicate local name '{local_name}'"),
                import.file,
                import.node.span,
            ));
            continue;
        }
        owner.imports.insert(
            local_name.clone(),
            ResolvedImport {
                local_name,
                definition: definition.id,
                span: import.node.span,
            },
        );
    }
    dependency_sites
}

fn module_definition(module: ModuleId, index: u32, file: &str, item: &Item) -> ModuleDefinition {
    let (name, kind, public, span) = match item {
        Item::Struct(node) => (
            &node.name,
            ModuleDefinitionKind::Struct,
            node.public,
            node.span,
        ),
        Item::Enum(node) => (
            &node.name,
            ModuleDefinitionKind::Enum,
            node.public,
            node.span,
        ),
        Item::Function(node) => (
            &node.name,
            ModuleDefinitionKind::Function,
            node.public,
            node.span,
        ),
        Item::State(node) => (
            &node.name,
            ModuleDefinitionKind::State,
            node.public,
            node.span,
        ),
        Item::Agent(node) => (
            &node.name,
            ModuleDefinitionKind::Agent,
            node.public,
            node.span,
        ),
        Item::Pipeline(node) => (
            &node.name,
            ModuleDefinitionKind::Pipeline,
            node.public,
            node.span,
        ),
        Item::SafetyBound(node) => (
            &node.name,
            ModuleDefinitionKind::SafetyBound,
            node.public,
            node.span,
        ),
    };
    ModuleDefinition {
        id: QualifiedDefId { module, index },
        name: name.clone(),
        kind,
        public,
        file: file.to_owned(),
        span,
    }
}

fn item_identity(item: &Item) -> (&str, Span) {
    match item {
        Item::Struct(node) => (&node.name, node.span),
        Item::Enum(node) => (&node.name, node.span),
        Item::Function(node) => (&node.name, node.span),
        Item::State(node) => (&node.name, node.span),
        Item::Agent(node) => (&node.name, node.span),
        Item::Pipeline(node) => (&node.name, node.span),
        Item::SafetyBound(node) => (&node.name, node.span),
    }
}

fn interface_hash(module: &ResolvedModule, sources: &[ModuleSource]) -> [u8; 32] {
    let mut signatures = Vec::new();
    for source in sources.iter().filter(|source| {
        source
            .ast
            .module
            .as_ref()
            .is_some_and(|declaration| declaration.path.join("::") == module.name)
    }) {
        signatures.extend(
            source
                .ast
                .items
                .iter()
                .filter(|item| item_public(item))
                .map(item_signature),
        );
    }
    signatures.sort();
    Sha256::digest(signatures.join("\n").as_bytes()).into()
}

fn item_public(item: &Item) -> bool {
    match item {
        Item::Struct(node) => node.public,
        Item::Enum(node) => node.public,
        Item::Function(node) => node.public,
        Item::State(node) => node.public,
        Item::Agent(node) => node.public,
        Item::Pipeline(node) => node.public,
        Item::SafetyBound(node) => node.public,
    }
}

fn item_signature(item: &Item) -> String {
    match item {
        Item::Function(node) => format!(
            "fn {}({})->{};async={}",
            node.name,
            node.parameters
                .iter()
                .map(|parameter| type_signature(&parameter.ty))
                .collect::<Vec<_>>()
                .join(","),
            node.return_type
                .as_ref()
                .map_or_else(|| "()".into(), type_signature),
            node.asynchronous
        ),
        Item::Struct(node) => format!(
            "struct {}{{{}}}",
            node.name,
            node.fields
                .iter()
                .map(|field| format!(
                    "{}:{}:{}",
                    field.public,
                    field.name,
                    type_signature(&field.ty)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Item::Enum(node) => format!(
            "enum {}{{{}}}",
            node.name,
            node.variants
                .iter()
                .map(|variant| format!(
                    "{}({})",
                    variant.name,
                    variant
                        .fields
                        .iter()
                        .map(type_signature)
                        .collect::<Vec<_>>()
                        .join(",")
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Item::State(node) => format!(
            "state {}:{:?}",
            node.name,
            node.fields
                .iter()
                .map(|field| (&field.name, type_signature(&field.ty)))
                .collect::<Vec<_>>()
        ),
        Item::Agent(node) => format!("agent {}", node.name),
        Item::Pipeline(node) => format!("pipeline {}", node.name),
        Item::SafetyBound(node) => format!("safety_bound {}", node.name),
    }
}

fn type_signature(ty: &TypeNode) -> String {
    match &ty.kind {
        TypeKind::Reference {
            lifetime,
            mutable,
            inner,
        } => format!(
            "&{}{}{}",
            lifetime
                .as_ref()
                .map_or_else(String::new, |name| format!("'{name} ")),
            if *mutable { "mut " } else { "" },
            type_signature(inner)
        ),
        TypeKind::Path {
            segments,
            arguments,
        } => {
            let mut result = segments.join("::");
            if !arguments.is_empty() {
                result.push('<');
                result.push_str(
                    &arguments
                        .iter()
                        .map(type_signature)
                        .collect::<Vec<_>>()
                        .join(","),
                );
                result.push('>');
            }
            result
        }
        TypeKind::Array(inner) => format!("[{}]", type_signature(inner)),
        TypeKind::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(type_signature)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn find_cycle(modules: &BTreeMap<ModuleId, ResolvedModule>) -> Option<Vec<ModuleId>> {
    fn visit(
        node: ModuleId,
        modules: &BTreeMap<ModuleId, ResolvedModule>,
        states: &mut BTreeMap<ModuleId, u8>,
        stack: &mut Vec<ModuleId>,
    ) -> Option<Vec<ModuleId>> {
        states.insert(node, 1);
        stack.push(node);
        let module = modules.get(&node)?;
        for neighbor in &module.dependencies {
            match states.get(neighbor).copied().unwrap_or(0) {
                0 => {
                    if let Some(cycle) = visit(*neighbor, modules, states, stack) {
                        return Some(cycle);
                    }
                }
                1 => {
                    let start = stack.iter().position(|candidate| candidate == neighbor)?;
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(*neighbor);
                    return Some(cycle);
                }
                _ => {}
            }
        }
        stack.pop();
        states.insert(node, 2);
        None
    }

    let mut states = BTreeMap::new();
    let mut stack = Vec::new();
    for id in modules.keys() {
        if states.get(id).copied().unwrap_or(0) == 0
            && let Some(cycle) = visit(*id, modules, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}

fn error(code: &str, message: String, file: String, span: Span) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: Severity::Error,
        message,
        file,
        span,
    }
}
