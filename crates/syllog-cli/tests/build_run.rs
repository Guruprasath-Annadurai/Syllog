//! Real artifact build and sandboxed execution CLI contracts.

use std::process::Command;

#[test]
fn build_writes_wasm_and_run_executes_main() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    let source = directory.path().join("main.syl");
    let artifact = directory.path().join("main.wasm");
    std::fs::write(&source, "fn main() -> I64 { 42 }\n").expect("source should be written");

    let build = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args([
            "build",
            source.to_str().unwrap(),
            "--target",
            "wasm32-syllog",
            "--output",
            artifact.to_str().unwrap(),
        ])
        .output()
        .expect("build command should launch");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(
        &std::fs::read(&artifact).expect("artifact should exist")[..4],
        b"\0asm"
    );

    let run = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args([
            "run",
            source.to_str().unwrap(),
            "--fuel",
            "100000",
            "--memory-bytes",
            "65536",
        ])
        .output()
        .expect("run command should launch");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
}

#[test]
fn run_rejects_missing_main_and_enforces_fuel_and_memory_policy() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    let missing = directory.path().join("missing.syl");
    let invalid = directory.path().join("invalid.syl");
    let valid = directory.path().join("valid.syl");
    std::fs::write(&missing, "fn helper() -> I64 { 42 }\n").expect("source should be written");
    std::fs::write(&valid, "fn main() -> I64 { 42 }\n").expect("source should be written");
    std::fs::write(&invalid, "fn main(value: U64) -> U64 { value }\n")
        .expect("source should be written");

    for args in [
        vec!["run", missing.to_str().unwrap()],
        vec!["run", invalid.to_str().unwrap()],
        vec!["run", valid.to_str().unwrap(), "--fuel", "1"],
        vec!["run", valid.to_str().unwrap(), "--memory-bytes", "0"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args(args)
            .output()
            .expect("run command should launch");
        assert!(
            !output.status.success(),
            "unexpected stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn manifest_schema_is_available_as_clean_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["schema", "manifest"])
        .output()
        .expect("schema command should launch");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let schema: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("schema output should be JSON");
    assert_eq!(schema["additionalProperties"], false);
}
