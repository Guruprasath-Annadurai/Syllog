//! Explicit async state-machine lowering contracts.

use syllog_compiler::{compile, lower_async_state_machines, lower_to_hir};
use syllog_ir::{AsyncTransition, verify_async_machine};

fn machines(source: &str) -> Vec<syllog_ir::AsyncStateMachine> {
    let compilation = compile("async.syl", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:#?}",
        compilation.diagnostics
    );
    let hir = lower_to_hir(
        compilation.ast.as_ref().unwrap(),
        compilation.symbols.as_ref().unwrap(),
    )
    .unwrap();
    lower_async_state_machines(&hir).unwrap()
}

#[test]
fn one_await_lowers_to_verified_suspend_resume_and_shared_drop_path() {
    let machines = machines(
        r"
        fn ready(value: U64) -> U64 { value }
        async fn job() -> U64 { await ready(7) }
        ",
    );
    assert_eq!(machines.len(), 1);
    let machine = &machines[0];
    verify_async_machine(machine).unwrap();
    assert_eq!(machine.states.len(), 5);
    assert!(matches!(
        machine.states[0].transition,
        AsyncTransition::Start { .. }
    ));
    assert!(matches!(
        machine.states[1].transition,
        AsyncTransition::Suspend { await_index: 0, .. }
    ));
    assert!(matches!(
        machine.states[2].transition,
        AsyncTransition::Resume { .. }
    ));
    assert!(matches!(
        machine.states[3].transition,
        AsyncTransition::Complete
    ));
    assert!(matches!(
        machine.states[4].transition,
        AsyncTransition::Cancel { .. }
    ));
}

#[test]
fn multiple_awaits_preserve_source_order_and_share_one_cancel_state() {
    let machines = machines(
        r"
        fn ready(value: U64) -> U64 { value }
        async fn job() -> U64 {
            let first: U64 = await ready(1)
            await ready(first)
        }
        ",
    );
    let machine = &machines[0];
    verify_async_machine(machine).unwrap();
    let await_indexes = machine
        .states
        .iter()
        .filter_map(|state| match state.transition {
            AsyncTransition::Suspend { await_index, .. } => Some(await_index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(await_indexes, [0, 1]);
    assert_eq!(
        machine
            .states
            .iter()
            .filter(|state| matches!(state.transition, AsyncTransition::Cancel { .. }))
            .count(),
        1
    );
}

#[test]
fn await_outside_async_function_is_rejected_at_await_expression() {
    let report = compile(
        "sync.syl",
        "fn ready(value: U64) -> U64 { value } fn bad() -> U64 { await ready(1) }",
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        ["SYL2501"]
    );
}
