//! Incremental rebuild, debounce, and shutdown contracts.

use std::time::Duration;

use syllog_dev_server::{DevEvent, DevOptions, serve};

#[tokio::test]
async fn editing_one_source_emits_one_sequence_without_rebuilding_unrelated_target() {
    let directory = tempfile::tempdir().expect("temporary project should exist");
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("Syllog.toml"),
        r#"[package]
name = "dev-test"
version = "0.1.0"
edition = "2026"

[[targets]]
name = "app"
kind = "bin"
path = "src/main.syl"

[[targets]]
name = "worker"
kind = "lib"
path = "src/worker.syl"

[capabilities]
profile = "none"
network = []
environment = []
max_memory_bytes = 65536
"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/main.syl"),
        "fn main() -> I64 { 1 }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/worker.syl"),
        "fn worker() -> I64 { 2 }\n",
    )
    .unwrap();
    let project = syllog_project::discover(directory.path()).unwrap();
    let mut handle = serve(
        project,
        DevOptions {
            poll_interval: Duration::from_millis(10),
            debounce: Duration::from_millis(30),
        },
    )
    .await
    .unwrap();

    let mut initial_restarts = 0;
    while initial_restarts < 2 {
        let event = tokio::time::timeout(Duration::from_secs(2), handle.next_event())
            .await
            .expect("initial builds should complete")
            .expect("event channel should remain open");
        if event == DevEvent::RuntimeRestarted {
            initial_restarts += 1;
        }
    }
    let before = handle.stats();
    std::fs::write(
        directory.path().join("src/main.syl"),
        "fn main() -> I64 { 3 }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/main.syl"),
        "fn main() -> I64 { 4 }\n",
    )
    .unwrap();

    let mut sequence = Vec::new();
    while sequence.last() != Some(&DevEvent::RuntimeRestarted) {
        let event = tokio::time::timeout(Duration::from_secs(2), handle.next_event())
            .await
            .expect("debounced rebuild should complete")
            .expect("event channel should remain open");
        sequence.push(event);
    }

    assert!(matches!(
        sequence.as_slice(),
        [
            DevEvent::Building(_),
            DevEvent::Diagnostics(_),
            DevEvent::Ready(_),
            DevEvent::RuntimeRestarted,
        ]
    ));
    let after = handle.stats();
    assert_eq!(after.total_builds, before.total_builds + 1);
    assert_eq!(after.by_target["worker"], before.by_target["worker"]);
    assert_eq!(after.by_target["app"], before.by_target["app"] + 1);

    handle
        .shutdown()
        .await
        .expect("shutdown should be graceful");
}
