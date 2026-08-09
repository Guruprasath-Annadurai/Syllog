//! Affine ownership, borrowing, and lexical region contracts.

use syllog_semantic::check_syl;

fn codes(source: &str) -> Vec<String> {
    check_syl("ownership.syl", source)
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn rejects_use_after_move_and_double_move() {
    let diagnostics = codes(
        r#"
fn consume() {
    let value: String = "owned"
    let first: String = value
    let second: String = value
}
"#,
    );
    assert!(diagnostics.contains(&"SYL2602".into()), "{diagnostics:?}");
}

#[test]
fn rejects_mutable_aliases_and_moves_while_borrowed() {
    let aliases = codes(
        r#"
fn aliases() {
    let value: String = "owned"
    let exclusive = &mut value
    let overlapping = &value
    exclusive
}
"#,
    );
    assert!(aliases.contains(&"SYL2603".into()), "{aliases:?}");

    let moved = codes(
        r#"
fn moved() {
    let value: String = "owned"
    let shared = &value
    let consumed: String = value
    shared
}
"#,
    );
    assert!(moved.contains(&"SYL2605".into()), "{moved:?}");
}

#[test]
fn rejects_local_borrow_escape_and_ambiguous_public_regions() {
    let local = codes(
        r#"
fn escape() -> &String {
    let value: String = "owned"
    return &value
}
"#,
    );
    assert!(local.contains(&"SYL2604".into()), "{local:?}");

    let ambiguous = codes("pub fn choose(left: &String, right: &String) -> &String { left }");
    assert!(ambiguous.contains(&"SYL2604".into()), "{ambiguous:?}");
}

#[test]
fn branch_join_marks_values_moved_on_any_path() {
    let diagnostics = codes(
        r#"
fn branch(flag: Bool) {
    let value: String = "owned"
    let selected: String = match flag {
        true => value,
        false => "fallback",
    }
    let invalid: String = value
}
"#,
    );
    assert!(diagnostics.contains(&"SYL2602".into()), "{diagnostics:?}");
}

#[test]
fn accepts_shared_aliases_and_non_lexical_reborrows() {
    let shared = codes(
        r#"
fn shared() {
    let value: String = "owned"
    let left = &value
    let right = &value
    left
    right
    return
}
"#,
    );
    assert!(
        !shared.iter().any(|code| code.starts_with("SYL26")),
        "{shared:?}"
    );

    let reborrow = codes(
        r#"
fn reborrow() {
    let value: String = "owned"
    let exclusive = &mut value
    exclusive
    let shared = &value
    shared
    return
}
"#,
    );
    assert!(
        !reborrow.iter().any(|code| code.starts_with("SYL26")),
        "{reborrow:?}"
    );
}

#[test]
fn named_output_regions_must_match_an_input_region() {
    let valid = codes("pub fn identity(value: &'a String) -> &'a String { value }");
    assert!(
        !valid.iter().any(|code| code.starts_with("SYL26")),
        "{valid:?}"
    );

    let invalid = codes("pub fn identity(value: &'a String) -> &'b String { value }");
    assert!(invalid.contains(&"SYL2604".into()), "{invalid:?}");
}

#[test]
fn rejects_borrows_that_cross_await_suspension() {
    let diagnostics = codes(
        r"
        fn ready(value: &String) -> &String { value }
        async fn unsafe_suspend(value: String) -> &String {
            let borrowed = &value
            await ready(borrowed)
        }
        ",
    );
    assert!(
        diagnostics.iter().any(|code| code == "SYL2606"),
        "{diagnostics:?}"
    );
}

#[test]
fn lexical_shadowing_restores_the_outer_ownership_state() {
    let diagnostics = codes(
        r#"
        fn shadow(value: String) {
            {
                let value: String = "inner"
                let consumed: String = value
            }
            let first: String = value
            let second: String = value
        }
        "#,
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|code| code.as_str() == "SYL2602")
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn user_algebraic_values_are_affine_without_an_explicit_copy_trait() {
    let diagnostics = codes(
        r"
        struct Record { enabled: Bool }
        fn consume(value: Record) {
            let first: Record = value
            let second: Record = value
        }
        ",
    );
    assert!(
        diagnostics.iter().any(|code| code == "SYL2602"),
        "{diagnostics:?}"
    );
}

#[test]
fn inferred_bindings_preserve_affine_ownership() {
    let diagnostics = codes(
        r"
        fn inferred(value: String) {
            let moved = value
            let first = moved
            let second = moved
        }
        ",
    );
    assert!(
        diagnostics.iter().any(|code| code == "SYL2602"),
        "{diagnostics:?}"
    );
}
