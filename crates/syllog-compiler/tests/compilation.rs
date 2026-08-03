//! Compilation phase and diagnostic rendering behavior.

use syllog_compiler::{CompilationPhase, EditorReport, compile, render_human};

#[test]
fn successful_compilation_runs_every_frontend_phase() {
    let source = include_str!("../../../examples/semantic_frontend.syl");
    let compilation = compile("examples/semantic_frontend.syl", source);

    assert!(compilation.success(), "{:#?}", compilation.diagnostics);
    assert_eq!(
        compilation.completed_phases,
        [
            CompilationPhase::Parse,
            CompilationPhase::Validate,
            CompilationPhase::Resolve,
            CompilationPhase::TypeCheck,
        ]
    );
    assert!(compilation.ast.is_some());
    assert!(
        compilation
            .symbols
            .as_ref()
            .is_some_and(|symbols| symbols.contains_value("decide"))
    );
}

#[test]
fn syntax_failure_stops_before_resolution() {
    let compilation = compile("broken.syl", "fn broken( {");

    assert!(!compilation.success());
    assert_eq!(compilation.completed_phases, [CompilationPhase::Parse]);
    assert!(compilation.ast.is_none());
    assert!(compilation.symbols.is_none());
    assert_eq!(compilation.diagnostics[0].phase, CompilationPhase::Parse);
}

#[test]
fn resolution_and_type_diagnostics_retain_their_phase() {
    let source = r#"
enum Color { red, blue }
fn unresolved(value: Missing) -> U64 { absent(value) }
fn label(color: Color) -> String {
    match color { Color::red => "red" }
}
"#;
    let compilation = compile("phases.syl", source);
    let phases: Vec<_> = compilation
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.phase)
        .collect();

    assert!(phases.contains(&CompilationPhase::Resolve));
    assert!(phases.contains(&CompilationPhase::TypeCheck));
    assert_eq!(
        compilation.completed_phases.last(),
        Some(&CompilationPhase::TypeCheck)
    );
}

#[test]
fn human_renderer_includes_context_line_caret_and_phase() {
    let source = "fn broken(value: U64) -> U64 { absent(value) }\n";
    let compilation = compile("src/broken.syl", source);
    let rendered = render_human(source, &compilation.diagnostics);

    assert!(
        rendered.contains("error[SYL2003]: unknown value 'absent'"),
        "{rendered}"
    );
    assert!(rendered.contains("--> src/broken.syl:1:32"), "{rendered}");
    assert!(rendered.contains("1 | fn broken(value: U64)"), "{rendered}");
    assert!(rendered.contains("^^^^^^"), "{rendered}");
    assert!(rendered.contains("= phase: resolve"), "{rendered}");
}

#[test]
fn editor_report_has_a_versioned_machine_only_schema() {
    let source = "agent bad {\n    provider: 42\n    context_window: 128000\n}\n";
    let compilation = compile("bad.syl", source);
    let report = EditorReport::from(&compilation);
    let json = serde_json::to_value(report).expect("editor report must serialize");

    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["success"], false);
    assert_eq!(json["diagnostics"][0]["code"], "SYL1201");
    assert_eq!(json["diagnostics"][0]["phase"], "validate");
    assert_eq!(json["diagnostics"][0]["range"]["start"]["line"], 1);
    assert_eq!(json["diagnostics"][0]["range"]["start"]["column"], 4);
    assert!(json.get("ast").is_none());
    assert!(json.get("symbols").is_none());
}
