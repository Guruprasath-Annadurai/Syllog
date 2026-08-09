//! Wasmtime execution policy contracts.

use std::borrow::Cow;
use std::collections::BTreeSet;

use syllog_codegen_wasm::{WasmOptions, emit_with_capabilities};
use syllog_ir::{
    BasicBlock, CapabilityManifest, Constant, DefId, Effect, LocalId, MirFunction, MirProgram,
    MirType, Operand, Place, Rvalue, Statement, Terminator,
};
use syllog_runtime::{HostCapability, Sandbox, SandboxError, SandboxPolicy};

fn wasm(source: &str) -> Vec<u8> {
    wat::parse_str(source).expect("test module must be valid WAT")
}

#[test]
fn executes_a_typed_export_inside_the_policy_store() {
    let sandbox = Sandbox::new().expect("engine should initialize");
    let policy = SandboxPolicy::new(10_000, 64 * 1024).expect("policy should be valid");
    let module = wasm(r#"(module (func (export "answer") (result i32) i32.const 42))"#);

    assert_eq!(
        sandbox
            .execute_i32(&module, "answer", &policy)
            .expect("bounded module should execute"),
        42
    );
}

#[test]
fn infinite_module_stops_with_a_typed_fuel_error() {
    let sandbox = Sandbox::new().expect("engine should initialize");
    let policy = SandboxPolicy::new(1_000, 64 * 1024).expect("policy should be valid");
    let module = wasm(
        r#"(module
            (func (export "spin") (result i32)
                (loop $forever br $forever)
                i32.const 0))"#,
    );

    let error = sandbox
        .execute_i32(&module, "spin", &policy)
        .expect_err("infinite module must consume its finite fuel");

    assert!(matches!(error, SandboxError::FuelExhausted));
}

#[test]
fn module_minimum_memory_cannot_exceed_policy() {
    let sandbox = Sandbox::new().expect("engine should initialize");
    let policy = SandboxPolicy::new(10_000, 64 * 1024).expect("policy should be valid");
    let module = wasm(
        r#"(module
            (memory 2)
            (func (export "answer") (result i32) i32.const 42))"#,
    );

    let error = sandbox
        .execute_i32(&module, "answer", &policy)
        .expect_err("two pages must not fit in a one-page policy");

    assert!(matches!(error, SandboxError::MemoryLimitExceeded));
}

#[test]
fn runtime_memory_growth_cannot_exceed_policy() {
    let sandbox = Sandbox::new().expect("engine should initialize");
    let policy = SandboxPolicy::new(10_000, 64 * 1024).expect("policy should be valid");
    let module = wasm(
        r#"(module
            (memory 1)
            (func (export "grow") (result i32)
                i32.const 1
                memory.grow))"#,
    );

    let error = sandbox
        .execute_i32(&module, "grow", &policy)
        .expect_err("growth beyond one page must be denied");

    assert!(matches!(error, SandboxError::MemoryLimitExceeded));
}

#[test]
fn host_import_is_denied_without_an_explicit_capability() {
    let sandbox = Sandbox::new().expect("engine should initialize");
    let policy = SandboxPolicy::new(10_000, 64 * 1024).expect("policy should be valid");
    let module = wasm(
        r#"(module
            (import "syllog" "log_i32" (func $log (param i32)))
            (func (export "answer") (result i32)
                i32.const 7
                call $log
                i32.const 42))"#,
    );

    let error = sandbox
        .execute_i32(&module, "answer", &policy)
        .expect_err("ambient host access must be denied");

    assert!(matches!(
        error,
        SandboxError::CapabilityDenied { module, name }
            if module == "syllog" && name == "log_i32"
    ));
}

#[test]
fn explicitly_granted_capability_links_only_its_host_function() {
    let sandbox = Sandbox::new().expect("engine should initialize");
    let policy = SandboxPolicy::new(10_000, 64 * 1024)
        .expect("policy should be valid")
        .allow(HostCapability::LogI32);
    let module = wasm(
        r#"(module
            (import "syllog" "log_i32" (func $log (param i32)))
            (func (export "answer") (result i32)
                i32.const 7
                call $log
                i32.const 42))"#,
    );

    assert_eq!(
        sandbox
            .execute_i32(&module, "answer", &policy)
            .expect("granted logging capability should link"),
        42
    );
}

fn capability_artifact(effect: Effect) -> Vec<u8> {
    let id = DefId {
        module: 0,
        index: 0,
    };
    let program = MirProgram {
        entry: Some(id),
        functions: vec![MirFunction {
            id,
            parameter_count: 0,
            return_type: MirType::U64,
            locals: vec![MirType::U64],
            blocks: vec![BasicBlock {
                statements: vec![Statement::Assign {
                    destination: Place::Local(LocalId(0)),
                    value: Rvalue::Use(Operand::Constant(Constant::U64(42))),
                }],
                terminator: Some(Terminator::Return),
            }],
        }],
    };
    emit_with_capabilities(
        &program,
        &[],
        &CapabilityManifest {
            format_version: 1,
            required: BTreeSet::from([effect]),
        },
        &WasmOptions::default(),
    )
    .unwrap()
    .bytes
}

#[test]
fn artifact_effects_are_denied_unless_the_policy_grants_them() {
    let sandbox = Sandbox::new().unwrap();
    let module = capability_artifact(Effect::Network);
    let denied = SandboxPolicy::new(10_000, 64 * 1024).unwrap();
    assert!(matches!(
        sandbox.execute_i64(&module, "main", &denied),
        Err(SandboxError::EffectDenied { effect: "network" })
    ));

    let granted = SandboxPolicy::new(10_000, 64 * 1024)
        .unwrap()
        .allow_effect(Effect::Network);
    assert_eq!(sandbox.execute_i64(&module, "main", &granted).unwrap(), 42);
}

#[test]
fn malformed_and_duplicate_effect_manifests_fail_closed() {
    let policy = SandboxPolicy::new(10_000, 64 * 1024).unwrap();
    let sandbox = Sandbox::new().unwrap();
    let mut malformed = wasm_encoder::Module::new();
    malformed.section(&wasm_encoder::CustomSection {
        name: Cow::Borrowed("syllog.capabilities"),
        data: Cow::Borrowed(b"not-json"),
    });
    assert!(matches!(
        sandbox.execute_i64(&malformed.finish(), "main", &policy),
        Err(SandboxError::InvalidCapabilityManifest { .. })
    ));

    let encoded = serde_json::to_vec(&CapabilityManifest::default()).unwrap();
    let mut duplicate = wasm_encoder::Module::new();
    for _ in 0..2 {
        duplicate.section(&wasm_encoder::CustomSection {
            name: Cow::Borrowed("syllog.capabilities"),
            data: Cow::Borrowed(&encoded),
        });
    }
    assert!(matches!(
        sandbox.execute_i64(&duplicate.finish(), "main", &policy),
        Err(SandboxError::InvalidCapabilityManifest { .. })
    ));
}
