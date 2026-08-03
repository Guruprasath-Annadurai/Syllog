//! Domain-specific configuration validation after syntax parsing.

use crate::{AgentNode, Ast, Diagnostic, Expr, ExprKind, Item, Literal, Property, Severity, Span};
use std::collections::HashSet;

pub(crate) fn validate(file: &str, ast: &Ast) -> Vec<Diagnostic> {
    let agents: HashSet<&str> = ast
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Agent(agent) => Some(agent.name.as_str()),
            _ => None,
        })
        .collect();
    let mut diagnostics = Vec::new();

    for item in &ast.items {
        match item {
            Item::Agent(agent) => validate_agent(file, agent, &mut diagnostics),
            Item::Pipeline(pipeline) => {
                duplicate_properties(file, &pipeline.fields, &mut diagnostics);
                let Some(reference) = required_property(
                    file,
                    "pipeline",
                    &pipeline.name,
                    pipeline.span,
                    &pipeline.fields,
                    "agent",
                    &mut diagnostics,
                ) else {
                    continue;
                };
                validate_agent_reference(file, reference, &agents, &mut diagnostics);
            }
            Item::SafetyBound(bound) => {
                duplicate_properties(file, &bound.fields, &mut diagnostics);
                if property(&bound.fields, "require").is_none()
                    && property(&bound.fields, "policy").is_none()
                {
                    diagnostics.push(error(
                        file,
                        "SYL1002",
                        "safety_bound requires a 'require' or 'policy' property",
                        bound.span,
                    ));
                }
            }
            _ => {}
        }
    }

    diagnostics
}

fn validate_agent(file: &str, agent: &AgentNode, diagnostics: &mut Vec<Diagnostic>) {
    duplicate_properties(file, &agent.fields, diagnostics);
    let provider = required_property(
        file,
        "agent",
        &agent.name,
        agent.span,
        &agent.fields,
        "provider",
        diagnostics,
    );
    required_property(
        file,
        "agent",
        &agent.name,
        agent.span,
        &agent.fields,
        "context_window",
        diagnostics,
    );

    if let Some(provider) = provider {
        match &provider.value.kind {
            ExprKind::Literal(Literal::String(name)) if !name.is_empty() => {
                validate_top_level_model(file, agent, diagnostics);
            }
            expression => match provider_call_status(expression) {
                ProviderCallStatus::Valid { has_model: true } => {}
                ProviderCallStatus::Valid { has_model: false } => {
                    validate_top_level_model(file, agent, diagnostics);
                }
                ProviderCallStatus::NotCall | ProviderCallStatus::Malformed => {
                    diagnostics.push(error(
                        file,
                        "SYL1201",
                        "provider must be a non-empty route string or a provider call with named arguments and a non-empty string model",
                        provider.span,
                    ));
                }
            },
        }
    }

    if let Some(fallback) = property(&agent.fields, "fallback") {
        match &fallback.value.kind {
            ExprKind::Array(entries) => {
                for entry in entries {
                    if !is_fallback_entry(entry) {
                        diagnostics.push(error(
                            file,
                            "SYL1202",
                            "fallback entry must be a non-empty route string or a provider call with a named string 'model' argument",
                            entry.span,
                        ));
                    }
                }
            }
            _ => diagnostics.push(error(
                file,
                "SYL1202",
                "fallback must be an array of provider definitions",
                fallback.value.span,
            )),
        }
    }
}

fn validate_top_level_model(file: &str, agent: &AgentNode, diagnostics: &mut Vec<Diagnostic>) {
    let model = required_property(
        file,
        "agent",
        &agent.name,
        agent.span,
        &agent.fields,
        "model",
        diagnostics,
    );
    if let Some(model) = model
        && !is_nonempty_string(&model.value)
    {
        diagnostics.push(error(
            file,
            "SYL1201",
            "agent 'model' must be a non-empty string",
            model.value.span,
        ));
    }
}

fn validate_agent_reference(
    file: &str,
    reference: &Property,
    agents: &HashSet<&str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let name = match &reference.value.kind {
        ExprKind::Literal(Literal::Identifier(name)) => Some(name.as_str()),
        ExprKind::Path(segments) => segments.last().map(String::as_str),
        _ => None,
    };
    match name {
        Some(name) if !agents.contains(name) => diagnostics.push(error(
            file,
            "SYL1101",
            format!("pipeline references unknown agent '{name}'"),
            reference.value.span,
        )),
        None => diagnostics.push(error(
            file,
            "SYL1101",
            "pipeline 'agent' must reference a declared agent by name",
            reference.value.span,
        )),
        Some(_) => {}
    }
}

fn duplicate_properties(file: &str, fields: &[Property], diagnostics: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for field in fields {
        if !seen.insert(field.name.as_str()) {
            diagnostics.push(error(
                file,
                "SYL1001",
                format!("duplicate property '{}'", field.name),
                field.span,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn required_property<'a>(
    file: &str,
    kind: &str,
    declaration: &str,
    declaration_span: Span,
    fields: &'a [Property],
    name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<&'a Property> {
    let found = property(fields, name);
    if found.is_none() {
        diagnostics.push(error(
            file,
            "SYL1002",
            format!("{kind} '{declaration}' is missing required property '{name}'"),
            declaration_span,
        ));
    }
    found
}

fn property<'a>(fields: &'a [Property], name: &str) -> Option<&'a Property> {
    fields.iter().find(|field| field.name == name)
}

fn is_fallback_entry(expression: &Expr) -> bool {
    is_nonempty_string(expression)
        || matches!(
            provider_call_status(&expression.kind),
            ProviderCallStatus::Valid { has_model: true }
        )
}

fn is_nonempty_string(expression: &Expr) -> bool {
    matches!(&expression.kind, ExprKind::Literal(Literal::String(value)) if !value.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderCallStatus {
    NotCall,
    Malformed,
    Valid { has_model: bool },
}

fn provider_call_status(expression: &ExprKind) -> ProviderCallStatus {
    let ExprKind::Call { callee, arguments } = expression else {
        return ProviderCallStatus::NotCall;
    };
    if !matches!(
        callee.kind,
        ExprKind::Literal(Literal::Identifier(_)) | ExprKind::Path(_)
    ) || arguments.iter().any(|argument| argument.name.is_none())
    {
        return ProviderCallStatus::Malformed;
    }

    let mut names = HashSet::new();
    if arguments
        .iter()
        .filter_map(|argument| argument.name.as_deref())
        .any(|name| !names.insert(name))
    {
        return ProviderCallStatus::Malformed;
    }

    let model = arguments
        .iter()
        .find(|argument| argument.name.as_deref() == Some("model"));
    match model {
        Some(model) if is_nonempty_string(&model.value) => {
            ProviderCallStatus::Valid { has_model: true }
        }
        Some(_) => ProviderCallStatus::Malformed,
        None => ProviderCallStatus::Valid { has_model: false },
    }
}

fn error(file: &str, code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        severity: Severity::Error,
        message: message.into(),
        file: file.to_owned(),
        span,
    }
}
