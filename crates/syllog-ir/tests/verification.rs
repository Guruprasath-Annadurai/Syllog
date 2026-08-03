//! MIR verifier rejection contracts.

use syllog_ir::{
    BasicBlock, BlockId, DefId, LocalId, MirFunction, MirProgram, MirType, Operand, Place, Rvalue,
    Statement, Terminator, VerificationError, verify,
};

fn function(blocks: Vec<BasicBlock>, locals: Vec<MirType>, result: MirType) -> MirProgram {
    MirProgram {
        functions: vec![MirFunction {
            id: DefId {
                module: 0,
                index: 0,
            },
            parameter_count: 0,
            return_type: result,
            locals,
            blocks,
        }],
    }
}

#[test]
fn rejects_a_missing_terminator() {
    let program = function(
        vec![BasicBlock {
            statements: Vec::new(),
            terminator: None,
        }],
        vec![MirType::Unit],
        MirType::Unit,
    );

    let errors = verify(&program).expect_err("unterminated block must be rejected");
    assert!(matches!(
        errors.as_slice(),
        [VerificationError::MissingTerminator { .. }]
    ));
}

#[test]
fn rejects_an_invalid_control_flow_target() {
    let program = function(
        vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Goto(BlockId(9))),
        }],
        vec![MirType::Unit],
        MirType::Unit,
    );

    let errors = verify(&program).expect_err("invalid target must be rejected");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::InvalidBlockTarget {
            target: BlockId(9),
            ..
        }
    )));
}

#[test]
fn rejects_use_before_definition() {
    let program = function(
        vec![BasicBlock {
            statements: vec![Statement::Assign {
                destination: Place::Local(LocalId(0)),
                value: Rvalue::Use(Operand::Copy(LocalId(1))),
            }],
            terminator: Some(Terminator::Return),
        }],
        vec![MirType::U64, MirType::U64],
        MirType::U64,
    );

    let errors = verify(&program).expect_err("undefined local use must be rejected");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::UseBeforeDefinition {
            local: LocalId(1),
            ..
        }
    )));
}

#[test]
fn rejects_return_destination_type_corruption() {
    let program = function(
        vec![BasicBlock {
            statements: vec![Statement::Assign {
                destination: Place::Local(LocalId(0)),
                value: Rvalue::Use(Operand::Constant(syllog_ir::Constant::U64(42))),
            }],
            terminator: Some(Terminator::Return),
        }],
        vec![MirType::U64],
        MirType::I64,
    );

    let errors = verify(&program).expect_err("corrupt return local type must be rejected");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::TypeMismatch {
            expected: MirType::I64,
            actual: MirType::U64,
            ..
        }
    )));
}
