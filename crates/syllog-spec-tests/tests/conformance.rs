//! Language conformance manifest contracts.

use std::path::Path;
use syllog_spec_tests::{ExpectedOutcome, load_cases};

#[test]
fn load_cases_preserves_expected_diagnostic_codes() {
    let repository = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository path should resolve");
    let cases = load_cases(&repository.join("spec/cases")).expect("manifest should load");

    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].edition, "2026");
    assert_eq!(
        cases[0].source,
        repository.join("spec/cases/semantics/unknown_value.syl")
    );
    assert_eq!(
        cases[0].expected,
        ExpectedOutcome::Diagnostics(vec!["SYL2003".into()])
    );
    assert_eq!(
        cases[1].source,
        repository.join("spec/cases/syntax/minimal_pass.syl")
    );
    assert_eq!(cases[1].expected, ExpectedOutcome::Pass);
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
                { "edition": "2026", "source": "case.syl", "expected": { "kind": "pass" } },
                { "edition": "2026", "source": "case.syl", "expected": { "kind": "pass" } }
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
