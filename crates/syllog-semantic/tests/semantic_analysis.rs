//! End-to-end semantic analysis behavior.

use syllog_parser::parse_syl;
use syllog_semantic::analyze;

fn codes(source: &str) -> Vec<String> {
    let ast = parse_syl(source).expect("semantic fixture must parse");
    analyze("semantic.syl", &ast)
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn builds_type_and_value_symbol_tables_with_forward_references() {
    let source = r"
fn lookup(id: U64) -> Option<User> { none }
struct User { id: U64 }
enum Lookup { found(User), missing }
";
    let ast = parse_syl(source).expect("fixture must parse");
    let analysis = analyze("symbols.syl", &ast);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.symbols.contains_type("Bool"));
    assert!(analysis.symbols.contains_type("User"));
    assert!(analysis.symbols.contains_type("Lookup"));
    assert!(analysis.symbols.contains_value("lookup"));
}

#[test]
fn reports_duplicate_symbols_unknown_types_and_unknown_values() {
    let source = r"
struct User {}
enum User { missing }
fn duplicate() {}
fn duplicate() {}
fn broken(value: MissingType) -> U64 { absent(value) }
";
    assert_eq!(codes(source), ["SYL2001", "SYL2001", "SYL2002", "SYL2003"]);
}

#[test]
fn enforces_option_and_result_type_arity() {
    let source = r"
state Invalid {
    first: Option<String, U64> = none
    second: Result<String> = none
}
";
    assert_eq!(codes(source), ["SYL2004", "SYL2004"]);
}

#[test]
fn checks_pipeline_types_against_the_selected_agent_contract() {
    let source = r#"
enum Outcome { accepted, rejected }
agent worker {
    provider: openai(model: "gpt-5")
    context_window: 128000
    input: String = request
    output: Outcome = response
}
pipeline broken(input: U64) -> Bool {
    agent: AgentRef = worker
    result: Bool = true
}
"#;
    let ast = parse_syl(source).expect("fixture must parse");
    let analysis = analyze("pipeline.syl", &ast);

    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        ["SYL2201", "SYL2201"]
    );
    assert!(analysis.diagnostics[0].message.contains("input"));
    assert!(analysis.diagnostics[1].message.contains("output"));
}

#[test]
fn reports_missing_variants_in_non_exhaustive_matches() {
    let source = r#"
enum Color { red, green, blue }
fn label(color: Color) -> String {
    match color {
        Color::red => "red",
        Color::green => "green",
    }
}
"#;
    let ast = parse_syl(source).expect("fixture must parse");
    let analysis = analyze("match.syl", &ast);

    assert_eq!(analysis.diagnostics.len(), 1);
    assert_eq!(analysis.diagnostics[0].code, "SYL2301");
    assert!(analysis.diagnostics[0].message.contains("blue"));
    assert_eq!(analysis.diagnostics[0].span.line, 4);
}

#[test]
fn option_result_and_wildcard_matches_are_exhaustive() {
    let source = r#"
struct Failure {}
enum Color { red, green }
fn option_text(value: Option<String>) -> String {
    match value {
        Option::some(text) => text,
        Option::none => "none",
    }
}
fn result_text(value: Result<String, Failure>) -> String {
    match value {
        Result::ok(text) => text,
        Result::err(_) => "error",
    }
}
fn color_text(value: Color) -> String {
    match value { _ => "color" }
}
"#;
    assert!(codes(source).is_empty());
}

#[test]
fn non_exhaustive_option_match_reports_none() {
    let source = r"
fn unwrap(value: Option<String>) -> String {
    match value { Option::some(text) => text }
}
";
    let ast = parse_syl(source).expect("fixture must parse");
    let analysis = analyze("option.syl", &ast);

    assert_eq!(analysis.diagnostics.len(), 1);
    assert_eq!(analysis.diagnostics[0].code, "SYL2301");
    assert!(analysis.diagnostics[0].message.contains("none"));
}

#[test]
fn checked_in_frontend_examples_pass_semantic_analysis() {
    for source in [
        include_str!("../../../examples/hello_agent.syl"),
        include_str!("../../../examples/core_frontend.syl"),
        include_str!("../../../examples/semantic_frontend.syl"),
    ] {
        let ast = parse_syl(source).expect("checked-in example must parse");
        let analysis = analyze("example.syl", &ast);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn option_and_result_constructors_produce_their_algebraic_types() {
    let source = r"
struct Failure {}
fn some(value: String) -> Option<String> { Option::some(value) }
fn ok(value: String) -> Result<String, Failure> { Result::ok(value) }
fn err(error: Failure) -> Result<String, Failure> { Result::err(error) }
";
    assert!(codes(source).is_empty());
}

#[test]
fn unit_tuple_syntax_resolves_to_the_unit_primitive() {
    let ast = parse_syl("fn unit() -> () {}").expect("fixture must parse");
    let analysis = analyze("unit.syl", &ast);
    assert!(analysis.symbols.contains_type("Unit"));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}
