//! End-to-end reference semantics from Syllog source through MIR.

use syllog_compiler::{lower_to_hir, lower_to_mir};
use syllog_interpreter::{InterpreterLimits, RuntimeError, RuntimeValue, execute};
use syllog_parser::parse_syl;
use syllog_semantic::analyze;

fn compile(source: &str) -> (syllog_ir::MirProgram, syllog_ir::DefId) {
    let ast = parse_syl(source).expect("interpreter fixture should parse");
    let analysis = analyze("execution.syl", &ast);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let hir = lower_to_hir(&ast, &analysis.symbols).expect("fixture should lower to HIR");
    let entry = hir.entry.expect("fixture should declare main");
    let mir = lower_to_mir(&hir).expect("fixture should lower to MIR");
    (
        mir,
        syllog_ir::DefId {
            module: entry.module.0,
            index: entry.index,
        },
    )
}

#[test]
fn executes_literal_main() {
    let (program, entry) = compile("fn main() -> U64 { 42 }");

    let result = execute(&program, entry, InterpreterLimits::default())
        .expect("literal program should execute");

    assert_eq!(result.value, RuntimeValue::U64(42));
    assert!(result.stdout.is_empty());
}

#[test]
fn executes_calls_arithmetic_and_exhaustive_enum_branching() {
    let (program, entry) = compile(
        r"
enum Choice { left, right }
fn score(choice: Choice) -> U64 {
    match choice {
        Choice::left => 20,
        Choice::right => 40,
    }
}
fn increment(value: U64) -> U64 { value + 2 }
fn main() -> U64 { increment(score(Choice::right)) }
",
    );

    let result = execute(&program, entry, InterpreterLimits::default())
        .expect("calls and branching should execute");

    assert_eq!(result.value, RuntimeValue::U64(42));
}

#[test]
fn instruction_budget_is_a_hard_deterministic_limit() {
    let (program, entry) = compile("fn main() -> U64 { 42 }");
    let limits = InterpreterLimits {
        max_instructions: 1,
        ..InterpreterLimits::default()
    };

    let error = execute(&program, entry, limits).expect_err("tiny budget must stop execution");

    assert_eq!(error, RuntimeError::InstructionLimitExceeded { limit: 1 });
}
