//! Typed HIR to verified MIR lowering contracts.

use syllog_compiler::{lower_to_hir, lower_to_mir};
use syllog_ir::{LocalId, Place, Rvalue, Statement, Terminator, verify};
use syllog_parser::parse_syl;
use syllog_semantic::analyze;

fn mir(source: &str) -> syllog_ir::MirProgram {
    let ast = parse_syl(source).expect("MIR fixture should parse");
    let analysis = analyze("mir.syl", &ast);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let hir = lower_to_hir(&ast, &analysis.symbols).expect("fixture should lower to HIR");
    lower_to_mir(&hir).expect("fixture should lower to verified MIR")
}

#[test]
fn lowers_constants_arithmetic_locals_and_calls_to_explicit_mir() {
    let program = mir(r"
fn increment(value: U64) -> U64 { value + 1 }
fn main() -> U64 {
    let start: U64 = 41
    increment(start)
}
");

    verify(&program).expect("compiler-produced MIR must verify");
    assert_eq!(program.functions.len(), 2);
    assert!(
        program
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .any(|block| {
                block.statements.iter().any(|statement| {
                    matches!(
                        statement,
                        Statement::Assign {
                            value: Rvalue::Binary { .. },
                            ..
                        }
                    )
                })
            })
    );
    assert!(
        program
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .any(|block| { matches!(block.terminator, Some(Terminator::Call { .. })) })
    );
}

#[test]
fn lowers_enum_construction_and_match_to_switch_control_flow() {
    let program = mir(r"
enum Choice { left, right }
fn choose(value: Choice) -> U64 {
    match value {
        Choice::left => 10,
        Choice::right => 20,
    }
}
");

    verify(&program).expect("match MIR must verify");
    let choose = &program.functions[0];
    assert!(choose.blocks.len() >= 4);
    assert!(
        choose
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, Some(Terminator::SwitchInt { .. })))
    );
    assert!(
        choose
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(
                statement,
                Statement::Assign {
                    value: Rvalue::Discriminant(_),
                    ..
                }
            ))
    );
}

#[test]
fn inserts_one_drop_for_each_owned_value_remaining_at_return() {
    let program = mir(r#"
        fn consume(value: String) -> U64 {
            let temporary: String = "owned"
            42
        }
        "#);
    let drops = program.functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            Statement::Drop(Place::Local(local)) => Some(*local),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(drops.len(), 2, "parameter and local must each drop once");
    assert!(drops.contains(&LocalId(1)));
    assert!(drops.iter().all(|local| *local != LocalId(0)));
    verify(&program).expect("drop-inserted MIR must satisfy affine verification");
}
