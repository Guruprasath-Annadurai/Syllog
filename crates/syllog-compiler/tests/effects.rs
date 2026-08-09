//! Whole-program effect inference contracts.

use std::collections::BTreeSet;

use syllog_compiler::{CompilationPhase, analyze_effects, compile, lower_to_hir};
use syllog_ir::Effect;

fn effects(source: &str) -> BTreeSet<Effect> {
    let compilation = compile("effects.syl", source);
    assert!(compilation.success(), "{:#?}", compilation.diagnostics);
    let hir = lower_to_hir(
        compilation.ast.as_ref().unwrap(),
        compilation.symbols.as_ref().unwrap(),
    )
    .unwrap();
    analyze_effects(&hir).unwrap().manifest.required
}

#[test]
fn infers_alloc_async_and_transitive_call_effects() {
    let inferred = effects(
        r#"
        fn text() -> String !{alloc} { "hello" }
        async fn task() -> String !{alloc, async} { text() }
        fn main() -> U64 !{pure} { 42 }
        "#,
    );
    assert_eq!(inferred, BTreeSet::from([Effect::Alloc, Effect::Async]));
}

#[test]
fn rejects_unknown_mixed_and_underdeclared_effect_bounds() {
    for (source, fragment) in [
        ("fn main() -> U64 !{magic} { 1 }", "unknown effect"),
        (
            "fn main() -> String !{pure, alloc} { \"x\" }",
            "cannot be combined",
        ),
        (
            "fn main() -> String !{pure} { \"x\" }",
            "omits inferred effect",
        ),
    ] {
        let compilation = compile("invalid-effects.syl", source);
        assert!(!compilation.success());
        assert!(compilation.diagnostics.iter().any(|diagnostic| {
            diagnostic.phase == CompilationPhase::EffectCheck
                && diagnostic.message.contains(fragment)
        }));
    }
}
