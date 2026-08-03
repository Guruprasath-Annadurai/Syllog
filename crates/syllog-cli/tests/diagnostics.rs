//! End-to-end CLI diagnostic behavior.

use std::fs;
use std::process::Command;

#[test]
fn check_exits_unsuccessfully_and_prints_structured_diagnostics() {
    let directory =
        std::env::temp_dir().join(format!("syllog-cli-domain-test-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("temporary test directory");
    let source_file = directory.join("bad-provider.syl");
    fs::write(
        &source_file,
        "agent bad {\n    provider: 42\n    context_window: 128000\n}\n",
    )
    .expect("temporary Syllog source");

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["check", source_file.to_str().expect("UTF-8 test path")])
        .output()
        .expect("syllog process must start");

    let stderr = String::from_utf8(output.stderr).expect("diagnostics must be UTF-8");
    assert!(
        !output.status.success(),
        "invalid configuration was accepted"
    );
    assert!(stderr.contains("bad-provider.syl:2:5"), "{stderr}");
    assert!(stderr.contains("error[SYL1201]"), "{stderr}");
    assert!(stderr.contains("2 |     provider: 42"), "{stderr}");
    assert!(stderr.contains("^^^^^^^^^^^^"), "{stderr}");

    fs::remove_file(&source_file).expect("remove temporary source");
    fs::remove_dir(&directory).expect("remove temporary directory");
}

#[test]
fn check_rejects_semantically_unresolved_programs() {
    let directory =
        std::env::temp_dir().join(format!("syllog-cli-semantic-test-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("temporary test directory");
    let source_file = directory.join("unknown-name.syl");
    fs::write(
        &source_file,
        "fn broken(value: U64) -> U64 { absent(value) }\n",
    )
    .expect("temporary Syllog source");

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["check", source_file.to_str().expect("UTF-8 test path")])
        .output()
        .expect("syllog process must start");

    let stderr = String::from_utf8(output.stderr).expect("diagnostics must be UTF-8");
    assert!(!output.status.success(), "unresolved function was accepted");
    assert!(stderr.contains("unknown-name.syl:1:"), "{stderr}");
    assert!(stderr.contains("SYL2003"), "{stderr}");

    fs::remove_file(&source_file).expect("remove temporary source");
    fs::remove_dir(&directory).expect("remove temporary directory");
}

#[test]
fn json_diagnostics_are_machine_clean_and_versioned() {
    let directory =
        std::env::temp_dir().join(format!("syllog-cli-json-test-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("temporary test directory");
    let source_file = directory.join("json-error.syl");
    fs::write(
        &source_file,
        "fn broken(value: U64) -> U64 { absent(value) }\n",
    )
    .expect("temporary Syllog source");

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args([
            "check",
            source_file.to_str().expect("UTF-8 test path"),
            "--diagnostic-format=json",
        ])
        .output()
        .expect("syllog process must start");

    assert!(!output.status.success());
    assert!(output.stderr.is_empty(), "JSON mode polluted stderr");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must contain only JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["success"], false);
    assert_eq!(report["diagnostics"][0]["code"], "SYL2003");
    assert_eq!(report["diagnostics"][0]["phase"], "resolve");

    fs::remove_file(&source_file).expect("remove temporary source");
    fs::remove_dir(&directory).expect("remove temporary directory");
}

#[test]
fn successful_json_check_contains_no_human_status_text() {
    let directory =
        std::env::temp_dir().join(format!("syllog-cli-json-ok-test-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("temporary test directory");
    let source_file = directory.join("valid.syl");
    fs::write(&source_file, "fn unit() -> () {}\n").expect("temporary Syllog source");

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args([
            "check",
            source_file.to_str().expect("UTF-8 test path"),
            "--json",
        ])
        .output()
        .expect("syllog process must start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must contain only JSON");
    assert_eq!(report["success"], true);
    assert_eq!(report["diagnostics"], serde_json::json!([]));

    fs::remove_file(&source_file).expect("remove temporary source");
    fs::remove_dir(&directory).expect("remove temporary directory");
}
