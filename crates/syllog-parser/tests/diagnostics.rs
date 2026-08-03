//! Behavioral coverage for source and domain diagnostics.

use syllog_parser::{Severity, check_syl};

#[test]
fn syntax_diagnostics_carry_filename_and_exact_source_coordinates() {
    let checked = check_syl("configs/broken.syl", "\nagent missing_name {");

    assert!(checked.ast.is_none());
    assert_eq!(checked.diagnostics.len(), 1);
    let diagnostic = &checked.diagnostics[0];
    assert_eq!(diagnostic.code, "SYL0001");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.file, "configs/broken.syl");
    assert_eq!(diagnostic.span.line, 2);
    assert!(diagnostic.span.column > 0);
    assert!(diagnostic.span.end >= diagnostic.span.start);
    assert!(diagnostic.to_string().contains("configs/broken.syl:2:"));
}

#[test]
fn duplicate_and_missing_properties_are_accumulated() {
    let source = r#"
agent incomplete {
    provider: "openai"
    provider: "anthropic"
}

pipeline unrouted {
    output: stream
}

safety_bound empty {
    enforced: true
}
"#;
    let checked = check_syl("domain.syl", source);
    let codes: Vec<_> = checked
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();

    assert!(
        checked.ast.is_some(),
        "semantic errors retain the parsed AST"
    );
    assert_eq!(
        codes,
        ["SYL1001", "SYL1002", "SYL1002", "SYL1002", "SYL1002"]
    );
    assert_eq!(checked.diagnostics[0].span.line, 4);
    assert!(
        checked
            .diagnostics
            .iter()
            .all(|error| error.file == "domain.syl")
    );
}

#[test]
fn unknown_agent_references_are_reported_at_the_property() {
    let source = r#"
agent known {
    provider: openai(model: "gpt-5")
    context_window: 128000
}
pipeline answer {
    agent: missing
}
"#;
    let checked = check_syl("routing.syl", source);

    assert_eq!(checked.diagnostics.len(), 1);
    assert_eq!(checked.diagnostics[0].code, "SYL1101");
    assert_eq!(checked.diagnostics[0].span.line, 7);
    assert!(checked.diagnostics[0].message.contains("missing"));
}

#[test]
fn malformed_provider_and_each_bad_fallback_entry_are_diagnosed() {
    let source = r#"
agent malformed {
    provider: 42
    context_window: 128000
    fallback: [42, openai("positional"), local(model: 7)]
}
"#;
    let checked = check_syl("providers.syl", source);
    let codes: Vec<_> = checked
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();

    assert_eq!(codes, ["SYL1201", "SYL1202", "SYL1202", "SYL1202"]);
    assert_eq!(checked.diagnostics[0].span.line, 3);
    assert!(
        checked.diagnostics[1..]
            .iter()
            .all(|diagnostic| diagnostic.span.line == 5)
    );
}

#[test]
fn checked_in_frontend_examples_have_no_configuration_diagnostics() {
    for (file, source) in [
        (
            "examples/hello_agent.syl",
            include_str!("../../../examples/hello_agent.syl"),
        ),
        (
            "examples/core_frontend.syl",
            include_str!("../../../examples/core_frontend.syl"),
        ),
    ] {
        let checked = check_syl(file, source);
        assert!(
            checked.diagnostics.is_empty(),
            "unexpected diagnostics for {file}: {:#?}",
            checked.diagnostics
        );
    }
}

#[test]
fn provider_call_may_use_the_agents_top_level_model_property() {
    let source = r#"
agent configured {
    provider: openai(api_key: secret("OPENAI_API_KEY"))
    model: "gpt-5"
    context_window: 128000
}
"#;
    let checked = check_syl("provider-model.syl", source);
    assert!(checked.diagnostics.is_empty(), "{:#?}", checked.diagnostics);
}
