//! Differential Wasm backend contracts.

use syllog_codegen_wasm::{WasmOptions, emit};
use syllog_compiler::{lower_to_hir, lower_to_mir};
use syllog_interpreter::{InterpreterLimits, RuntimeValue, execute};
use syllog_parser::parse_syl;
use syllog_runtime::Sandbox;
use syllog_semantic::analyze;

fn mir(source: &str) -> syllog_ir::MirProgram {
    let ast = parse_syl(source).expect("Wasm fixture should parse");
    let analysis = analyze("codegen.syl", &ast);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let hir = lower_to_hir(&ast, &analysis.symbols).expect("fixture should lower to HIR");
    lower_to_mir(&hir).expect("fixture should lower to MIR")
}

#[test]
fn wasm_and_reference_interpreter_match_hand_derived_result() {
    let program = mir(r"
enum Choice { left, right }
fn score(choice: Choice) -> U64 {
    match choice {
        Choice::left => 20,
        Choice::right => 40,
    }
}
fn increment(value: U64) -> U64 { value + 2 }
fn main() -> U64 { increment(score(Choice::right)) }
");
    let entry = program.entry.expect("main should become MIR entry");
    let interpreted = execute(&program, entry, InterpreterLimits::default())
        .expect("reference execution should succeed");
    assert_eq!(interpreted.value, RuntimeValue::U64(42));

    let artifact = emit(&program, &WasmOptions::default()).expect("Wasm emission should succeed");
    Sandbox::new()
        .expect("sandbox should initialize")
        .compile(&artifact.bytes)
        .expect("emitted module should validate in the production sandbox");
    let engine = wasmtime::Engine::default();
    let module = wasmtime::Module::new(&engine, &artifact.bytes).expect("Wasm should compile");
    let mut store = wasmtime::Store::new(&engine, ());
    let instance =
        wasmtime::Instance::new(&mut store, &module, &[]).expect("Wasm should instantiate");
    let main = instance
        .get_typed_func::<(), i64>(&mut store, "main")
        .expect("main should use the integer ABI");
    assert_eq!(main.call(&mut store, ()).expect("Wasm should execute"), 42);
}

#[test]
fn artifacts_are_byte_deterministic_and_versioned() {
    let program = mir("fn main() -> U64 { 42 }");

    let first = emit(&program, &WasmOptions::default()).expect("first emission should succeed");
    let second = emit(&program, &WasmOptions::default()).expect("second emission should succeed");

    assert_eq!(first, second);
    assert_eq!(first.metadata.format_version, 1);
    assert_eq!(first.metadata.entry, program.entry.unwrap());
    assert_ne!(first.metadata.source_hash, [0; 32]);
}
