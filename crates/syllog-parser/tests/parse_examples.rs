//! Integration coverage for checked-in Syllog source programs.

use syllog_parser::{ExprKind, Item, Literal, StatementKind, TypeKind, parse_syl};

#[test]
fn parses_the_checked_in_hello_agent_program() {
    // Regression target: deleting any top-level declaration or failing to
    // retain its typed fields must make this test fail.
    let source = include_str!("../../../examples/hello_agent.syl");
    let ast = parse_syl(source).expect("hello_agent.syl must parse");

    assert_eq!(ast.items.len(), 3);

    let Item::Agent(agent) = &ast.items[0] else {
        panic!("first item must be an agent")
    };
    assert_eq!(agent.name, "assistant");
    assert_eq!(
        agent.field("provider"),
        Some(&Literal::String("openai".into()))
    );
    assert_eq!(
        agent.field("context_window"),
        Some(&Literal::Integer(128_000))
    );

    let Item::Pipeline(pipeline) = &ast.items[1] else {
        panic!("second item must be a pipeline")
    };
    assert_eq!(pipeline.name, "answer_request");
    assert_eq!(
        pipeline.field("agent"),
        Some(&Literal::Identifier("assistant".into()))
    );

    let Item::SafetyBound(bound) = &ast.items[2] else {
        panic!("third item must be a safety bound")
    };
    assert_eq!(bound.name, "output_policy");
    assert_eq!(bound.field("enforced"), Some(&Literal::Boolean(true)));
}

#[test]
fn rejects_unknown_top_level_constructs() {
    let error = parse_syl("unknown thing { enabled: true }")
        .expect_err("unknown declarations must be rejected");
    assert!(
        error.to_string().contains("agent"),
        "unexpected diagnostic: {error}"
    );
}

#[test]
fn decodes_escaped_string_literals() {
    let ast =
        parse_syl(r#"agent a { system: "line\n\"quoted\"" }"#).expect("escaped strings must parse");
    let Item::Agent(agent) = &ast.items[0] else {
        panic!("expected agent")
    };
    assert_eq!(
        agent.field("system"),
        Some(&Literal::String("line\n\"quoted\"".into()))
    );
}

#[test]
fn parses_struct_enum_async_fn_state_expressions_and_typed_domain_properties() {
    let ast = parse_syl(include_str!("../../../examples/core_frontend.syl"))
        .expect("the expanded core surface must parse");
    assert_eq!(ast.items.len(), 7);

    let Item::Struct(user) = &ast.items[0] else {
        panic!("first item must be a struct")
    };
    assert!(user.public);
    assert_eq!(user.fields.len(), 2);
    assert!(matches!(user.fields[1].ty.kind, TypeKind::Array(_)));
    assert_span(user.span);
    assert_span(user.fields[1].span);
    assert_span(user.fields[1].ty.span);

    let Item::Enum(outcome) = &ast.items[1] else {
        panic!("second item must be an enum")
    };
    assert_eq!(outcome.variants.len(), 2);
    assert_eq!(outcome.variants[0].fields.len(), 1);
    assert_span(outcome.variants[0].span);

    let Item::State(store) = &ast.items[2] else {
        panic!("third item must be state")
    };
    assert_eq!(store.fields.len(), 2);
    assert!(matches!(
        store.fields[0].initializer.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::Array(items)) if items.is_empty()
    ));
    assert_span(store.fields[0].initializer.as_ref().unwrap().span);

    let Item::Function(resolve) = &ast.items[3] else {
        panic!("fourth item must be a function")
    };
    assert!(resolve.asynchronous);
    assert_eq!(resolve.parameters.len(), 1);
    assert_eq!(resolve.body.statements.len(), 2);
    assert!(matches!(
        resolve.body.statements[0].kind,
        StatementKind::Let { .. }
    ));
    let StatementKind::Expression(match_expr) = &resolve.body.statements[1].kind else {
        panic!("function tail must be a match expression")
    };
    let ExprKind::Match { arms, .. } = &match_expr.kind else {
        panic!("function tail must retain match syntax")
    };
    assert_eq!(arms.len(), 1);
    assert_span(resolve.body.span);
    assert_span(resolve.parameters[0].span);
    assert_span(arms[0].pattern.span);

    let Item::Agent(agent) = &ast.items[4] else {
        panic!("fifth item must be an agent")
    };
    let provider = agent.property("provider").expect("provider property");
    assert!(provider.ty.is_some());
    assert!(matches!(provider.value.kind, ExprKind::Call { .. }));
    let fallback = agent.property("fallback").expect("fallback property");
    assert!(matches!(fallback.value.kind, ExprKind::Array(_)));
    assert_span(provider.span);
    assert_span(provider.ty.as_ref().unwrap().span);
    assert_span(provider.value.span);

    let Item::Pipeline(pipeline) = &ast.items[5] else {
        panic!("sixth item must be a pipeline")
    };
    assert_eq!(pipeline.parameters.len(), 1);
    assert!(pipeline.return_type.is_some());
    assert!(pipeline.property("result").unwrap().ty.is_some());

    let Item::SafetyBound(bound) = &ast.items[6] else {
        panic!("seventh item must be a safety bound")
    };
    assert_eq!(bound.parameters.len(), 1);
    assert_eq!(bound.field("require"), Some(&Literal::Boolean(true)));
}

fn assert_span(span: syllog_parser::Span) {
    assert!(span.end > span.start, "empty span: {span:?}");
    assert!(span.line > 0 && span.column > 0, "invalid span: {span:?}");
}

#[test]
fn every_serialized_ast_node_exposes_a_source_span() {
    let ast = parse_syl(include_str!("../../../examples/hello_agent.syl"))
        .expect("hello_agent.syl must parse");
    let json = serde_json::to_value(ast).expect("AST must serialize");

    assert!(
        json.get("span").is_some(),
        "program span is missing: {json}"
    );
    for item in json["items"].as_array().expect("items must be an array") {
        let node = item.as_object().expect("tagged item must be an object");
        let payload = node.values().next().expect("item payload is missing");
        assert!(
            payload.get("span").is_some(),
            "item span is missing: {item}"
        );
    }
}

#[test]
fn preserves_postfix_precedence_named_calls_and_guarded_match_patterns() {
    let source = r"
fn compute(x: I64) -> Bool {
    match service.fetch(x, flags: [true, false]).status {
        Result::ok(value) if value > 0 && !false => true,
        _ => false,
    }
}
";
    let ast = parse_syl(source).expect("postfix and match expressions must parse");
    let Item::Function(function) = &ast.items[0] else {
        panic!("expected a function")
    };
    let StatementKind::Expression(expression) = &function.body.statements[0].kind else {
        panic!("expected a match expression statement")
    };
    let ExprKind::Match { value, arms } = &expression.kind else {
        panic!("expected retained match syntax")
    };
    assert!(matches!(value.kind, ExprKind::Field { .. }));
    assert_eq!(arms.len(), 2);
    assert!(arms[0].guard.is_some());
    assert_span(arms[0].span);
}

#[test]
fn keyword_prefixes_remain_identifiers() {
    let source = "fn agentCount(structure: U64) -> U64 { return structure }";
    let ast = parse_syl(source).expect("keyword prefixes are valid identifiers");
    let Item::Function(function) = &ast.items[0] else {
        panic!("expected a function")
    };
    assert_eq!(function.name, "agentCount");
    assert_eq!(function.parameters[0].name, "structure");
}
