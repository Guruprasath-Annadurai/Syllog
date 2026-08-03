//! CLI development-event protocol contracts.

use std::process::Command;

#[test]
fn dev_once_emits_machine_clean_ordered_events_and_exits() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    let create = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["new", "dev-app"])
        .current_dir(directory.path())
        .output()
        .expect("new command should launch");
    assert!(create.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["dev", "--once", "--json-events"])
        .current_dir(directory.path().join("dev-app"))
        .output()
        .expect("dev command should launch");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let kinds = events
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        ["building", "diagnostics", "ready", "runtime_restarted"]
    );
}
