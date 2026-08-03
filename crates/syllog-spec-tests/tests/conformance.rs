//! Language conformance manifest contracts.

use std::path::Path;
use syllog_spec_tests::{
    CasePolarity, ExpectedOutcome, load_cases, load_normative_rule_ids, validate_rule_coverage,
};

#[test]
fn load_cases_preserves_expected_diagnostic_codes() {
    let repository = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository path should resolve");
    let cases = load_cases(&repository.join("spec/cases")).expect("manifest should load");

    assert_eq!(cases.len(), 26);
    let unknown_value = cases
        .iter()
        .find(|case| case.source.ends_with("semantics/unknown_value.syl"))
        .expect("unknown-value case should exist");
    assert_eq!(unknown_value.edition, "2026");
    assert_eq!(
        unknown_value.source,
        repository.join("spec/cases/semantics/unknown_value.syl")
    );
    assert_eq!(
        unknown_value.expected,
        ExpectedOutcome::Diagnostics(vec!["SYL2003".into()])
    );
    assert_eq!(unknown_value.rules, ["SYL-VALUE-NAME-001"]);
    assert_eq!(unknown_value.polarity, CasePolarity::Negative);
    let minimal = cases
        .iter()
        .find(|case| case.source.ends_with("syntax/minimal_pass.syl"))
        .expect("minimal syntax case should exist");
    assert_eq!(
        minimal.source,
        repository.join("spec/cases/syntax/minimal_pass.syl")
    );
    assert_eq!(minimal.expected, ExpectedOutcome::Pass);
}

#[test]
fn load_cases_rejects_duplicate_edition_and_source() {
    let directory = tempfile::tempdir().expect("temporary conformance root should exist");
    std::fs::write(directory.path().join("case.syl"), "fn main() {}\n")
        .expect("fixture source should be written");
    std::fs::write(
        directory.path().join("manifest.json"),
        r#"{
            "schema_version": 1,
            "cases": [
                {
                    "edition": "2026",
                    "source": "case.syl",
                    "rules": ["SYL-SYNTAX-ITEM-001"],
                    "polarity": "positive",
                    "expected": { "kind": "pass" }
                },
                {
                    "edition": "2026",
                    "source": "case.syl",
                    "rules": ["SYL-SYNTAX-ITEM-001"],
                    "polarity": "positive",
                    "expected": { "kind": "pass" }
                }
            ]
        }"#,
    )
    .expect("fixture manifest should be written");

    let error = load_cases(directory.path()).expect_err("duplicate case must be rejected");

    assert!(
        error.to_string().contains("duplicate conformance case"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn compiler_outcomes_are_exact_and_deterministic() {
    let repository = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository path should resolve");
    let cases = load_cases(&repository.join("spec/cases")).expect("manifest should load");

    for case in cases {
        let source =
            std::fs::read_to_string(&case.source).expect("fixture source should be readable");
        let first = syllog_compiler::compile(case.source.display().to_string(), &source);
        let second = syllog_compiler::compile(case.source.display().to_string(), &source);
        let first_codes = first
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect::<Vec<_>>();
        let second_codes = second
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            first_codes,
            second_codes,
            "nondeterministic diagnostics for {}",
            case.source.display()
        );
        match case.expected {
            ExpectedOutcome::Pass => assert!(
                first.success(),
                "{} emitted {first_codes:?}",
                case.source.display()
            ),
            ExpectedOutcome::Diagnostics(expected) => assert_eq!(
                first_codes,
                expected,
                "unexpected diagnostics for {}",
                case.source.display()
            ),
            ExpectedOutcome::Run { stdout, exit_code } => {
                assert_runtime_case(&case.source, &first, &first_codes, &stdout, exit_code);
            }
        }
    }
}

fn assert_runtime_case(
    source: &Path,
    compilation: &syllog_compiler::Compilation,
    diagnostic_codes: &[String],
    stdout: &str,
    exit_code: i32,
) {
    assert!(
        compilation.success(),
        "{} emitted {diagnostic_codes:?}",
        source.display()
    );
    let ast = compilation
        .ast
        .as_ref()
        .expect("successful compilation retains AST");
    let symbols = compilation
        .symbols
        .as_ref()
        .expect("successful compilation retains symbols");
    let hir =
        syllog_compiler::lower_to_hir(ast, symbols).expect("runtime fixture should lower to HIR");
    let entry = hir.entry.expect("runtime fixture should declare main");
    let mir = syllog_compiler::lower_to_mir(&hir).expect("runtime fixture should lower to MIR");
    let result = syllog_interpreter::execute(
        &mir,
        syllog_ir::DefId {
            module: entry.module.0,
            index: entry.index,
        },
        syllog_interpreter::InterpreterLimits::default(),
    )
    .expect("runtime fixture should execute");
    assert_eq!(result.stdout, stdout.as_bytes(), "stdout mismatch");
    let actual_exit = match result.value {
        syllog_interpreter::RuntimeValue::Unit => 0,
        syllog_interpreter::RuntimeValue::I64(value) => {
            i32::try_from(value).expect("conformance exit value should fit i32")
        }
        syllog_interpreter::RuntimeValue::U64(value) => {
            i32::try_from(value).expect("conformance exit value should fit i32")
        }
        value => panic!("invalid main return value for process exit: {value:?}"),
    };
    assert_eq!(actual_exit, exit_code, "exit mismatch");
    let artifact = syllog_codegen_wasm::emit(&mir, &syllog_codegen_wasm::WasmOptions::default())
        .expect("runtime fixture should emit Wasm");
    let policy = syllog_runtime::SandboxPolicy::new(1_000_000, 64 * 1024)
        .expect("conformance sandbox policy should be valid");
    let wasm_exit = syllog_runtime::Sandbox::new()
        .expect("conformance sandbox should initialize")
        .execute_i64(&artifact.bytes, "main", &policy)
        .expect("runtime fixture Wasm should execute");
    assert_eq!(
        wasm_exit,
        i64::from(actual_exit),
        "interpreter/Wasm mismatch for {}",
        source.display()
    );
}

#[test]
fn every_normative_rule_has_positive_and_negative_fixtures() {
    let repository = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository path should resolve");
    let rules = load_normative_rule_ids(&repository.join("docs/language-reference.md"))
        .expect("normative rule identifiers should load");
    let cases = load_cases(&repository.join("spec/cases")).expect("manifest should load");

    let gaps = validate_rule_coverage(&rules, &cases);

    assert!(gaps.is_empty(), "uncovered normative rules: {gaps:#?}");
}
