//! Governance document completeness contract.

use std::path::Path;
use syllog_spec_tests::validate_governance;

#[test]
fn governance_documents_are_complete() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let issues = validate_governance(&repository).expect("governance validation should run");

    assert!(issues.is_empty(), "governance issues: {issues:#?}");
}
