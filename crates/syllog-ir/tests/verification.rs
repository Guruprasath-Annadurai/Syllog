//! MIR verifier rejection contracts.

use syllog_ir::{
    BasicBlock, BlockId, Constant, DefId, LocalId, MirFunction, MirProgram, MirType, Operand,
    Place, Rvalue, Statement, Terminator, VerificationError, verify,
};

fn function(blocks: Vec<BasicBlock>, locals: Vec<MirType>, result: MirType) -> MirProgram {
    MirProgram {
        entry: Some(DefId {
            module: 0,
            index: 0,
        }),
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

#[test]
fn rejects_use_after_affine_move() {
    let program = function(
        vec![BasicBlock {
            statements: vec![
                Statement::Assign {
                    destination: Place::Local(LocalId(1)),
                    value: Rvalue::Use(Operand::Constant(syllog_ir::Constant::String("x".into()))),
                },
                Statement::Assign {
                    destination: Place::Local(LocalId(2)),
                    value: Rvalue::Use(Operand::Move(LocalId(1))),
                },
                Statement::Assign {
                    destination: Place::Local(LocalId(3)),
                    value: Rvalue::Use(Operand::Move(LocalId(1))),
                },
            ],
            terminator: Some(Terminator::Return),
        }],
        vec![
            MirType::Unit,
            MirType::String,
            MirType::String,
            MirType::String,
        ],
        MirType::Unit,
    );

    let errors = verify(&program).expect_err("a moved local cannot be consumed twice");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::UseAfterMove {
            local: LocalId(1),
            ..
        }
    )));
}

#[test]
fn rejects_copy_and_double_drop_of_affine_values() {
    let program = function(
        vec![BasicBlock {
            statements: vec![
                Statement::Assign {
                    destination: Place::Local(LocalId(1)),
                    value: Rvalue::Use(Operand::Constant(syllog_ir::Constant::String("x".into()))),
                },
                Statement::Assign {
                    destination: Place::Local(LocalId(2)),
                    value: Rvalue::Use(Operand::Copy(LocalId(1))),
                },
                Statement::Drop(Place::Local(LocalId(1))),
                Statement::Drop(Place::Local(LocalId(1))),
            ],
            terminator: Some(Terminator::Return),
        }],
        vec![MirType::Unit, MirType::String, MirType::String],
        MirType::Unit,
    );

    let errors = verify(&program).expect_err("affine copy and repeated drop must be rejected");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::InvalidCopy {
            local: LocalId(1),
            ..
        }
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::DoubleDrop {
            local: LocalId(1),
            ..
        }
    )));
}

#[test]
fn loop_backedges_cannot_resurrect_moved_parameters() {
    let id = DefId {
        module: 0,
        index: 0,
    };
    let program = MirProgram {
        entry: Some(id),
        functions: vec![MirFunction {
            id,
            parameter_count: 1,
            return_type: MirType::Unit,
            locals: vec![MirType::Unit, MirType::String, MirType::String],
            blocks: vec![BasicBlock {
                statements: vec![Statement::Assign {
                    destination: Place::Local(LocalId(2)),
                    value: Rvalue::Use(Operand::Move(LocalId(1))),
                }],
                terminator: Some(Terminator::Goto(BlockId(0))),
            }],
        }],
    };

    let errors = verify(&program).expect_err("a loop cannot move its parameter repeatedly");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::UseAfterMove {
            local: LocalId(1),
            ..
        }
    )));
}

#[test]
fn rejects_overwriting_a_live_affine_local_without_drop() {
    let program = function(
        vec![BasicBlock {
            statements: vec![
                Statement::Assign {
                    destination: Place::Local(LocalId(1)),
                    value: Rvalue::Use(Operand::Constant(Constant::String("first".into()))),
                },
                Statement::Assign {
                    destination: Place::Local(LocalId(1)),
                    value: Rvalue::Use(Operand::Constant(Constant::String("second".into()))),
                },
            ],
            terminator: Some(Terminator::Return),
        }],
        vec![MirType::Unit, MirType::String],
        MirType::Unit,
    );

    let errors = verify(&program).expect_err("overwriting owned storage would leak its value");
    assert!(errors.iter().any(|error| matches!(
        error,
        VerificationError::OverwriteWithoutDrop {
            local: LocalId(1),
            ..
        }
    )));
}
