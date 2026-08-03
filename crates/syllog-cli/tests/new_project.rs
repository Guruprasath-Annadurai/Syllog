//! Deterministic, executable project scaffolding contracts.

use std::process::Command;

#[test]
fn new_basic_project_is_deterministic_and_immediately_checks_and_runs() {
    let directory = tempfile::tempdir().expect("temporary parent should exist");
    let create = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .args(["new", "frontier-agent", "--template", "basic"])
        .current_dir(directory.path())
        .output()
        .expect("new command should launch");
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );

    let project = directory.path().join("frontier-agent");
    assert_eq!(
        std::fs::read_to_string(project.join("Syllog.toml")).unwrap(),
        "[package]\nname = \"frontier-agent\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[[targets]]\nname = \"frontier-agent\"\nkind = \"bin\"\npath = \"src/main.syl\"\n\n[capabilities]\nprofile = \"none\"\nnetwork = []\nenvironment = []\nmax_memory_bytes = 65536\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.join(".syllog-template-version")).unwrap(),
        "basic@1\n"
    );

    for command in ["check", "run"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args([command, "src/main.syl"])
            .current_dir(&project)
            .output()
            .expect("generated behavior should execute");
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let tests = Command::new(env!("CARGO_BIN_EXE_syllog"))
        .arg("test")
        .current_dir(&project)
        .output()
        .expect("generated tests should execute");
    assert!(
        tests.status.success(),
        "{}",
        String::from_utf8_lossy(&tests.stderr)
    );
    assert!(String::from_utf8_lossy(&tests.stdout).contains("scaffold_smoke ... passed"));
}

#[test]
fn new_refuses_invalid_names_templates_and_existing_targets() {
    let directory = tempfile::tempdir().expect("temporary parent should exist");
    std::fs::create_dir(directory.path().join("occupied")).unwrap();
    std::fs::write(directory.path().join("occupied/keep.txt"), "user data").unwrap();

    for args in [
        ["new", "Bad_Name", "--template", "basic"],
        ["new", "demo", "--template", "unknown"],
        ["new", "occupied", "--template", "basic"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args(args)
            .current_dir(directory.path())
            .output()
            .expect("new command should launch");
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
    assert_eq!(
        std::fs::read_to_string(directory.path().join("occupied/keep.txt")).unwrap(),
        "user data"
    );
}

#[test]
fn every_advertised_template_produces_a_valid_discoverable_project() {
    let directory = tempfile::tempdir().expect("temporary parent should exist");
    for (template, capability_profile) in
        [("basic", "none"), ("agent", "agent"), ("native", "native")]
    {
        let name = format!("{template}-app");
        let output = Command::new(env!("CARGO_BIN_EXE_syllog"))
            .args(["new", &name, "--template", template])
            .current_dir(directory.path())
            .output()
            .expect("new command should launch");
        assert!(
            output.status.success(),
            "{template}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let project = syllog_project::discover(&directory.path().join(&name))
            .expect("generated manifest should validate");
        assert_eq!(project.manifest.package.name, name);
        assert_eq!(project.manifest.capabilities.profile, capability_profile);
    }
}
