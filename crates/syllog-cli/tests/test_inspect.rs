//! Project test runner and inspection contracts.

use std::process::Command;

fn project(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("Syllog.toml"),
        r#"[package]
name = "inspection-app"
version = "0.1.0"
edition = "2026"

[[targets]]
name = "app"
kind = "bin"
path = "src/main.syl"

[capabilities]
profile = "agent"
network = ["api.example.com:443"]
environment = ["SECRET_TOKEN"]
max_memory_bytes = 65536
"#,
    )
    .unwrap();
    std::fs::write(directory.path().join("src/main.syl"), source).unwrap();
    directory
}

#[test]
fn tests_are_discovered_ordered_sandboxed_and_fail_the_process() {
    let directory = project(
        "#[test]\nfn zeta() -> Bool { false }\n#[test]\nfn alpha() -> Bool { true }\nfn main() -> I64 { 0 }\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["test", "--json"])
        .current_dir(directory.path())
        .output()
        .expect("test command should launch");

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["success"], false);
    assert_eq!(report["tests"][0]["name"], "alpha");
    assert_eq!(report["tests"][0]["status"], "passed");
    assert_eq!(report["tests"][1]["name"], "zeta");
    assert_eq!(report["tests"][1]["status"], "failed");
}

#[test]
fn inspect_commands_are_stable_and_never_expand_environment_secrets() {
    let directory = project("#[test]\nfn works() -> Bool { true }\nfn main() -> I64 { 0 }\n");
    for subject in ["project", "capabilities", "hir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args(["inspect", subject, "--json"])
            .env("SECRET_TOKEN", "must-never-appear")
            .current_dir(directory.path())
            .output()
            .expect("inspect command should launch");
        assert!(
            output.status.success(),
            "{subject}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("must-never-appear"));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["schema_version"], 1, "{subject}");
    }
}
