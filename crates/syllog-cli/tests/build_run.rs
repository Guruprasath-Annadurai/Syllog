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

#[test]
fn project_build_and_run_compile_the_complete_module_tree() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("Syllog.toml"),
        r#"[package]
name = "multi-module"
version = "0.1.0"
edition = "2026"

[[targets]]
name = "multi-module"
kind = "bin"
path = "src/main.syl"

[capabilities]
profile = "none"
max_memory_bytes = 65536
"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/math.syl"),
        "module math;\npub fn answer() -> I64 { 42 }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/main.syl"),
        "module app;\nuse math::answer;\nfn main() -> I64 { twice(answer()) }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/helper.syl"),
        "module app;\npub fn twice(value: I64) -> I64 { value + value }\n",
    )
    .unwrap();
    let artifact = directory.path().join("target/app.wasm");

    let build = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .current_dir(directory.path())
        .args([
            "build",
            ".",
            "--target",
            "wasm32-syllog",
            "--output",
            artifact.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(&std::fs::read(&artifact).unwrap()[..4], b"\0asm");

    let run = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .current_dir(directory.path())
        .args(["run", ".", "--memory-bytes", "65536"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"84\n");
}

#[test]
fn project_build_exports_resumable_async_frame_steps() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("async-app");
    let created = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["new", "async-app", "--template", "basic"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(created.status.success());
    std::fs::write(
        root.join("src/main.syl"),
        "module app;\nfn ready(value: U64) -> U64 { value }\nasync fn job() -> U64 { await ready(7) }\nfn main() -> U64 { 0 }\n",
    )
    .unwrap();
    let artifact = root.join("target/async.wasm");
    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["build", ".", "--output", artifact.to_str().unwrap()])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let engine = wasmtime::Engine::default();
    let module = wasmtime::Module::from_file(&engine, artifact).unwrap();
    assert!(module.exports().any(
        |export| export.name().starts_with("syllog_async_") && export.name().ends_with("_step")
    ));
}
