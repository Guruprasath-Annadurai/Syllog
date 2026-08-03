//! Typed HIR lowering contracts.

use syllog_compiler::hir::{HirExprKind, HirStatement};
use syllog_compiler::lower_to_hir;
use syllog_parser::parse_syl;
use syllog_semantic::{ResolvedType, analyze};

#[test]
fn every_executable_expression_is_typed_and_every_reference_has_a_def_id() {
    let source = r"
enum Decision { allow, deny }

fn choose(flag: Bool) -> Decision {
    match flag {
        true => Decision::allow,
        false => Decision::deny,
    }
}

fn main() -> Decision {
    let enabled: Bool = true
    choose(enabled)
}
";
    let ast = parse_syl(source).expect("HIR fixture should parse");
    let analysis = analyze("hir.syl", &ast);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let program = lower_to_hir(&ast, &analysis.symbols).expect("valid source should lower");

    assert_eq!(program.schema_version, 1);
    assert!(program.entry.is_some(), "main must be the entry definition");
    let mut expression_count = 0;
    let mut reference_count = 0;
    for definition in &program.modules[0].definitions {
        let Some(function) = definition.kind.as_function() else {
            continue;
        };
        for statement in &function.body.statements {
            let expression = match statement {
                HirStatement::Let { value, .. } | HirStatement::Expression(value) => Some(value),
                HirStatement::Return(value) => value.as_ref(),
            };
            if let Some(expression) = expression {
                expression.walk(&mut |expression| {
                    expression_count += 1;
                    assert!(!matches!(
                        expression.ty,
                        ResolvedType::Unknown | ResolvedType::Error
                    ));
                    if let HirExprKind::Reference { definition } = expression.kind {
                        reference_count += 1;
                        assert_eq!(definition.module, program.modules[0].id);
                    }
                });
            }
        }
    }
    assert!(
        expression_count >= 8,
        "expected nested executable expressions"
    );
    assert!(
        reference_count >= 5,
        "expected local, function, and variant references"
    );
}

#[test]
fn hir_debug_serialization_is_versioned_and_deterministic() {
    let ast = parse_syl("fn main() -> U64 { 42 }").expect("fixture should parse");
    let analysis = analyze("main.syl", &ast);

    let first = lower_to_hir(&ast, &analysis.symbols).expect("fixture should lower");
    let second = lower_to_hir(&ast, &analysis.symbols).expect("fixture should lower twice");

    assert_eq!(first, second);
    let debug = serde_json::to_value(first).expect("HIR should serialize");
    assert_eq!(debug["schema_version"], 1);
}
