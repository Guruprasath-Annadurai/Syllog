//! Governance document completeness contract.

use std::path::Path;
use syllog_spec_tests::{validate_governance, validate_repository_truth};

#[test]
fn governance_documents_are_complete() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let issues = validate_governance(&repository).expect("governance validation should run");

    assert!(issues.is_empty(), "governance issues: {issues:#?}");
}

#[test]
fn repository_identity_ci_and_authority_are_truthful() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let issues =
        validate_repository_truth(&repository).expect("repository-truth validation should run");

    assert!(issues.is_empty(), "repository-truth issues: {issues:#?}");
}

#[test]
fn repository_truth_validator_rejects_missing_and_contradictory_inputs() {
    let repository = tempfile::tempdir().expect("temporary repository");
    std::fs::write(
        repository.path().join("Cargo.toml"),
        "repository = \"https://example.invalid/wrong\"\n",
    )
    .expect("write contradictory manifest");

    let issues = validate_repository_truth(repository.path())
        .expect("repository-truth validation should report failures");
    let codes = issues.iter().map(|issue| issue.code).collect::<Vec<_>>();

    assert!(codes.contains(&"repository.identity.cargo"));
    assert!(codes.contains(&"repository.identity.readme"));
    assert!(codes.contains(&"repository.toolchain.pin"));
    assert!(codes.contains(&"repository.toolchain.msrv"));
    assert!(codes.contains(&"repository.toolchain.ci"));
    assert!(codes.contains(&"repository.document.missing"));
    assert!(codes.contains(&"repository.ci.default_branch"));
    assert!(codes.contains(&"repository.authority.parser"));
}
