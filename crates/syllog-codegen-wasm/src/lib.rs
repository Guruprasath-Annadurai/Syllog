//! Verified MIR to deterministic WebAssembly code generation.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use syllog_ir::{
    BasicBlock, BinaryOp, Constant, DefId, LocalId, MirFunction, MirProgram, MirType, Operand,
    Place, Rvalue, Statement, Terminator, VerificationError, verify,
};
use wasm_encoder::{
    BlockType, CodeSection, CustomSection, ExportKind, ExportSection, Function, FunctionSection,
    Instruction, Module, TypeSection, ValType,
};

/// Stable metadata embedded beside one Wasm artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    /// Metadata and custom-section format.
    pub format_version: u32,
    /// Exported entry definition.
    pub entry: DefId,
    /// SHA-256 of canonical verified MIR input.
    pub source_hash: [u8; 32],
}

/// Deterministic Wasm emission controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmOptions {
    /// Include stable function names in debug metadata.
    pub debug_info: bool,
    /// Canonicalize floating NaNs when floating-point support is enabled.
    pub canonicalize_nan: bool,
}

impl Default for WasmOptions {
    fn default() -> Self {
        Self {
            debug_info: true,
            canonicalize_nan: true,
        }
    }
}

/// Deterministic Wasm artifact and provenance metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmArtifact {
    /// Valid WebAssembly module bytes.
    pub bytes: Vec<u8>,
    /// Versioned provenance.
    pub metadata: ArtifactMetadata,
}

/// Failure to translate verified MIR into the supported Wasm ABI.
#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    /// Input MIR violates a backend invariant.
    #[error("MIR verification failed")]
    InvalidMir(Vec<VerificationError>),
    /// Program declares no executable entry.
    #[error("MIR program has no entry function")]
    MissingEntry,
    /// Type has no representation in the current Wasm ABI.
    #[error("unsupported Wasm type {0:?}")]
    UnsupportedType(MirType),
    /// MIR feature is not supported by the current Wasm backend.
    #[error("unsupported MIR operation: {0}")]
    UnsupportedOperation(String),
    /// Canonical artifact metadata could not be encoded.
    #[error("failed to encode deterministic artifact metadata")]
    Encoding(#[source] serde_json::Error),
}

/// Emits a standalone Wasm module after re-verifying MIR.
///
/// # Errors
///
/// Returns verification, missing-entry, unsupported-ABI, or encoding errors.
pub fn emit(program: &MirProgram, options: &WasmOptions) -> Result<WasmArtifact, CodegenError> {
    verify(program).map_err(CodegenError::InvalidMir)?;
    let entry = program.entry.ok_or(CodegenError::MissingEntry)?;
    let function_indices = program
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| {
            Ok((
                function.id,
                u32::try_from(index)
                    .map_err(|_| CodegenError::UnsupportedOperation("too many functions".into()))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CodegenError>>()?;
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    for (index, function) in program.functions.iter().enumerate() {
        supported_signature(function)?;
        let parameters = vec![ValType::I64; function.parameter_count as usize];
        types.ty().function(parameters, [ValType::I64]);
        functions.function(
            u32::try_from(index).map_err(|_| {
                CodegenError::UnsupportedOperation("too many function types".into())
            })?,
        );
    }
    let mut exports = ExportSection::new();
    let entry_index = function_indices
        .get(&entry)
        .copied()
        .ok_or(CodegenError::MissingEntry)?;
    exports.export("main", ExportKind::Func, entry_index);

    let mut code = CodeSection::new();
    for function in &program.functions {
        code.function(&encode_function(function, &function_indices)?);
    }

    let canonical = serde_json::to_vec(program).map_err(CodegenError::Encoding)?;
    let source_hash: [u8; 32] = Sha256::digest(&canonical).into();
    let metadata = ArtifactMetadata {
        format_version: 1,
        entry,
        source_hash,
    };
    let source_map = encode_source_map(program, *options);
    let mut module = Module::new();
    module.section(&types);
    module.section(&functions);
    module.section(&exports);
    module.section(&code);
    module.section(&CustomSection {
        name: Cow::Borrowed("syllog.source_map"),
        data: Cow::Owned(source_map),
    });
    Ok(WasmArtifact {
        bytes: module.finish(),
        metadata,
    })
}

fn supported_signature(function: &MirFunction) -> Result<(), CodegenError> {
    for ty in &function.locals {
        match ty {
            MirType::Unit | MirType::Bool | MirType::I64 | MirType::U64 | MirType::Aggregate(_) => {
            }
            unsupported @ MirType::String => {
                return Err(CodegenError::UnsupportedType(unsupported.clone()));
            }
        }
    }
    Ok(())
}

fn encode_function(
    mir: &MirFunction,
    indices: &BTreeMap<DefId, u32>,
) -> Result<Function, CodegenError> {
    let parameter_count = mir.parameter_count;
    let non_parameters = u32::try_from(mir.locals.len())
        .map_err(|_| CodegenError::UnsupportedOperation("too many locals".into()))?
        .saturating_sub(parameter_count);
    let mut function = Function::new([(non_parameters.saturating_add(1), ValType::I64)]);
    let pc = u32::try_from(mir.locals.len())
        .map_err(|_| CodegenError::UnsupportedOperation("too many locals".into()))?;
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(pc));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    for (block_index, block) in mir.blocks.iter().enumerate() {
        function.instruction(&Instruction::LocalGet(pc));
        function
            .instruction(&Instruction::I64Const(i64::try_from(block_index).map_err(
                |_| CodegenError::UnsupportedOperation("too many blocks".into()),
            )?));
        function.instruction(&Instruction::I64Eq);
        function.instruction(&Instruction::If(BlockType::Empty));
        encode_block(&mut function, mir, block, pc, indices)?;
        function.instruction(&Instruction::End);
    }
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::End);
    Ok(function)
}

fn encode_block(
    function: &mut Function,
    mir: &MirFunction,
    block: &BasicBlock,
    pc: u32,
    indices: &BTreeMap<DefId, u32>,
) -> Result<(), CodegenError> {
    for statement in &block.statements {
        match statement {
            Statement::Assign { destination, value } => {
                encode_rvalue(function, mir, value)?;
                encode_set_place(function, mir, destination)?;
            }
            Statement::Drop(Place::Local(_)) => {}
            Statement::Drop(Place::Field { .. }) => {
                return Err(CodegenError::UnsupportedOperation(
                    "aggregate field drop".into(),
                ));
            }
        }
    }
    let terminator = block
        .terminator
        .as_ref()
        .expect("verified MIR has a terminator");
    match terminator {
        Terminator::Return => {
            function.instruction(&Instruction::LocalGet(wasm_local(mir, LocalId(0))));
            function.instruction(&Instruction::Return);
        }
        Terminator::Goto(target) => encode_continue(function, pc, target.0, 1),
        Terminator::Call {
            function: callee,
            args,
            destination,
            next,
        } => {
            for argument in args {
                encode_operand(function, mir, argument)?;
            }
            function.instruction(&Instruction::Call(indices[callee]));
            encode_set_place(function, mir, destination)?;
            encode_continue(function, pc, next.0, 1);
        }
        Terminator::SwitchInt {
            value,
            targets,
            otherwise,
        } => {
            for (candidate, target) in targets {
                encode_operand(function, mir, value)?;
                let candidate = u64::try_from(*candidate).map_err(|_| {
                    CodegenError::UnsupportedOperation("switch value exceeds the i64 ABI".into())
                })?;
                function.instruction(&Instruction::I64Const(i64_bits(candidate)));
                function.instruction(&Instruction::I64Eq);
                function.instruction(&Instruction::If(BlockType::Empty));
                encode_continue(function, pc, target.0, 2);
                function.instruction(&Instruction::End);
            }
            encode_continue(function, pc, otherwise.0, 1);
        }
    }
    Ok(())
}

fn encode_continue(function: &mut Function, pc: u32, target: u32, depth: u32) {
    function.instruction(&Instruction::I64Const(i64::from(target)));
    function.instruction(&Instruction::LocalSet(pc));
    function.instruction(&Instruction::Br(depth));
}

fn encode_rvalue(
    function: &mut Function,
    mir: &MirFunction,
    value: &Rvalue,
) -> Result<(), CodegenError> {
    match value {
        Rvalue::Use(operand) | Rvalue::Discriminant(operand) => {
            encode_operand(function, mir, operand)
        }
        Rvalue::Aggregate { discriminant, .. } => {
            function.instruction(&Instruction::I64Const(i64_bits(*discriminant)));
            Ok(())
        }
        Rvalue::Binary {
            operator,
            left,
            right,
        } => {
            encode_operand(function, mir, left)?;
            encode_operand(function, mir, right)?;
            let unsigned = operand_type(mir, left) == Some(MirType::U64);
            let instruction = match operator {
                BinaryOp::Add => Instruction::I64Add,
                BinaryOp::Subtract => Instruction::I64Sub,
                BinaryOp::Multiply => Instruction::I64Mul,
                BinaryOp::Divide if unsigned => Instruction::I64DivU,
                BinaryOp::Divide => Instruction::I64DivS,
                BinaryOp::Equal => Instruction::I64Eq,
                BinaryOp::Less if unsigned => Instruction::I64LtU,
                BinaryOp::Less => Instruction::I64LtS,
                BinaryOp::And => Instruction::I64And,
                BinaryOp::Or => Instruction::I64Or,
            };
            function.instruction(&instruction);
            Ok(())
        }
    }
}

fn encode_operand(
    function: &mut Function,
    mir: &MirFunction,
    operand: &Operand,
) -> Result<(), CodegenError> {
    match operand {
        Operand::Copy(local) | Operand::Move(local) => {
            function.instruction(&Instruction::LocalGet(wasm_local(mir, *local)));
        }
        Operand::Constant(constant) => {
            let value = match constant {
                Constant::Unit => 0,
                Constant::Bool(value) => i64::from(*value),
                Constant::I64(value) => *value,
                Constant::U64(value) => i64_bits(*value),
                Constant::String(_) => {
                    return Err(CodegenError::UnsupportedType(MirType::String));
                }
            };
            function.instruction(&Instruction::I64Const(value));
        }
    }
    Ok(())
}

fn encode_set_place(
    function: &mut Function,
    mir: &MirFunction,
    place: &Place,
) -> Result<(), CodegenError> {
    match place {
        Place::Local(local) => {
            function.instruction(&Instruction::LocalSet(wasm_local(mir, *local)));
            Ok(())
        }
        Place::Field { .. } => Err(CodegenError::UnsupportedOperation(
            "aggregate field assignment".into(),
        )),
    }
}

fn wasm_local(function: &MirFunction, local: LocalId) -> u32 {
    let parameters = function.parameter_count;
    if local.0 == 0 {
        parameters
    } else if local.0 <= parameters {
        local.0 - 1
    } else {
        local.0
    }
}

fn operand_type(function: &MirFunction, operand: &Operand) -> Option<MirType> {
    match operand {
        Operand::Constant(constant) => Some(constant.ty()),
        Operand::Copy(local) | Operand::Move(local) => {
            function.locals.get(local.0 as usize).cloned()
        }
    }
}

fn encode_source_map(program: &MirProgram, options: WasmOptions) -> Vec<u8> {
    let mut map = format!(
        "version=1;debug={};nan={};",
        options.debug_info, options.canonicalize_nan
    );
    for function in &program.functions {
        write!(
            map,
            "{}:{}:{};",
            function.id.module,
            function.id.index,
            function.blocks.len()
        )
        .expect("writing to String cannot fail");
    }
    map.into_bytes()
}

fn i64_bits(value: u64) -> i64 {
    i64::from_le_bytes(value.to_le_bytes())
}
