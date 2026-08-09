//! Whole-program effect inference and capability-manifest construction.

use std::collections::{BTreeMap, BTreeSet};

use syllog_ir::{CapabilityManifest, Effect};
use syllog_parser::{Literal, Span};

use crate::hir::{DefId, HirBlock, HirDefinitionKind, HirExprKind, HirProgram, HirStatement};

/// One source-positioned effect-system failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectError {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Offending declaration or expression.
    pub span: Span,
}

/// Result of whole-program effect inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectAnalysis {
    /// Inferred effects for every executable definition.
    pub functions: BTreeMap<DefId, BTreeSet<Effect>>,
    /// Conservative union embedded into the artifact.
    pub manifest: CapabilityManifest,
}

#[derive(Default)]
struct LocalEffects {
    direct: BTreeSet<Effect>,
    calls: BTreeSet<DefId>,
}

/// Infers effects transitively and verifies explicit effect upper bounds.
///
/// `pure` denotes the empty set and cannot be combined with another effect.
/// The resulting artifact manifest is deliberately conservative: it is the
/// union of all compiled executable definitions, including currently
/// unreachable public API, so deployment cannot accidentally under-grant.
///
/// # Errors
///
/// Returns source-positioned errors for malformed explicit bounds or when a
/// bound omits an effect inferred from the transitive call graph.
pub fn analyze_effects(program: &HirProgram) -> Result<EffectAnalysis, Vec<EffectError>> {
    let mut local = BTreeMap::<DefId, LocalEffects>::new();
    let mut declarations = BTreeMap::<DefId, (Option<BTreeSet<Effect>>, Span)>::new();
    let mut errors = Vec::new();

    for module in &program.modules {
        for definition in &module.definitions {
            match &definition.kind {
                HirDefinitionKind::Function(function) => {
                    let mut effects = LocalEffects::default();
                    if function.asynchronous {
                        effects.direct.insert(Effect::Async);
                    }
                    inspect_block(&function.body, &mut effects);
                    let declared = parse_declaration(
                        function.declared_effects.as_deref(),
                        definition.span,
                        &mut errors,
                    );
                    local.insert(definition.id, effects);
                    declarations.insert(definition.id, (declared, definition.span));
                }
                HirDefinitionKind::Pipeline(pipeline) => {
                    let mut effects = LocalEffects::default();
                    if pipeline.agent.is_some() {
                        effects.direct.extend([Effect::Network, Effect::Provider]);
                    }
                    if let Some(body) = &pipeline.body {
                        inspect_expr(body, &mut effects);
                    }
                    local.insert(definition.id, effects);
                }
                _ => {}
            }
        }
    }

    let mut inferred = local
        .iter()
        .map(|(id, effects)| (*id, effects.direct.clone()))
        .collect::<BTreeMap<_, _>>();
    loop {
        let snapshot = inferred.clone();
        let mut changed = false;
        for (id, effects) in &local {
            let target = inferred.entry(*id).or_default();
            for callee in &effects.calls {
                if let Some(callee_effects) = snapshot.get(callee) {
                    let old_len = target.len();
                    target.extend(callee_effects);
                    changed |= target.len() != old_len;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for (id, (declared, span)) in declarations {
        if let Some(declared) = declared {
            let actual = inferred.get(&id).cloned().unwrap_or_default();
            let missing = actual.difference(&declared).copied().collect::<Vec<_>>();
            if !missing.is_empty() {
                errors.push(EffectError {
                    code: "SYL2702",
                    message: format!(
                        "declared effect bound omits inferred effect(s): {}",
                        display_effects(&missing)
                    ),
                    span,
                });
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|error| (error.span.start, error.code));
        return Err(errors);
    }

    let required = inferred
        .values()
        .flat_map(|effects| effects.iter().copied())
        .collect();
    Ok(EffectAnalysis {
        functions: inferred,
        manifest: CapabilityManifest {
            format_version: 1,
            required,
        },
    })
}

fn parse_declaration(
    declaration: Option<&[String]>,
    span: Span,
    errors: &mut Vec<EffectError>,
) -> Option<BTreeSet<Effect>> {
    let declaration = declaration?;
    let pure = declaration.iter().any(|name| name == "pure");
    if pure && declaration.len() != 1 {
        errors.push(EffectError {
            code: "SYL2701",
            message: "'pure' cannot be combined with effect capabilities".into(),
            span,
        });
    }
    let mut parsed = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for name in declaration {
        if !seen.insert(name.as_str()) {
            errors.push(EffectError {
                code: "SYL2701",
                message: format!("duplicate effect '{name}'"),
                span,
            });
        } else if name != "pure" {
            match Effect::from_source(name) {
                Some(effect) => {
                    parsed.insert(effect);
                }
                None => errors.push(EffectError {
                    code: "SYL2701",
                    message: format!("unknown effect '{name}'"),
                    span,
                }),
            }
        }
    }
    Some(parsed)
}

fn inspect_block(block: &HirBlock, effects: &mut LocalEffects) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                inspect_expr(value, effects);
            }
            HirStatement::Return(value) => {
                if let Some(value) = value {
                    inspect_expr(value, effects);
                }
            }
        }
    }
}

fn inspect_expr(expression: &crate::hir::TypedExpr, effects: &mut LocalEffects) {
    match &expression.kind {
        HirExprKind::Await(operand) => {
            effects.direct.insert(Effect::Async);
            inspect_expr(operand, effects);
        }
        HirExprKind::Literal(Literal::String(_)) | HirExprKind::Array(_) => {
            effects.direct.insert(Effect::Alloc);
            if let HirExprKind::Array(items) = &expression.kind {
                for item in items {
                    inspect_expr(item, effects);
                }
            }
        }
        HirExprKind::Call { callee, arguments } => {
            if let HirExprKind::Reference { definition } = callee.kind {
                effects.calls.insert(definition);
            }
            inspect_expr(callee, effects);
            for argument in arguments {
                inspect_expr(argument, effects);
            }
        }
        HirExprKind::Borrow { operand, .. }
        | HirExprKind::Field { base: operand, .. }
        | HirExprKind::Unary { operand, .. } => inspect_expr(operand, effects),
        HirExprKind::Binary { left, right, .. } => {
            inspect_expr(left, effects);
            inspect_expr(right, effects);
        }
        HirExprKind::Match { value, arms } => {
            inspect_expr(value, effects);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    inspect_expr(guard, effects);
                }
                inspect_expr(&arm.body, effects);
            }
        }
        HirExprKind::Block(block) => inspect_block(block, effects),
        HirExprKind::Literal(_) | HirExprKind::Reference { .. } => {}
    }
}

fn display_effects(effects: &[Effect]) -> String {
    effects
        .iter()
        .map(|effect| effect.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
