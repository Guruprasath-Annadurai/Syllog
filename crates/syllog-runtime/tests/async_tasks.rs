//! Deterministic and Tokio async-machine execution contracts.

use syllog_ir::{AsyncState, AsyncStateId, AsyncStateMachine, AsyncTransition, DefId};
use syllog_runtime::{
    ParentScopeId, TaskControl, TaskEvent, TaskFailure, run_deterministic_task, run_tokio_task,
};

fn machine() -> AsyncStateMachine {
    let local = DefId {
        module: 0,
        index: 9,
    };
    AsyncStateMachine {
        function: DefId {
            module: 0,
            index: 1,
        },
        parent_scope_required: true,
        live_locals: vec![local],
        states: vec![
            AsyncState {
                id: AsyncStateId(0),
                transition: AsyncTransition::Start {
                    next: AsyncStateId(1),
                },
            },
            AsyncState {
                id: AsyncStateId(1),
                transition: AsyncTransition::Suspend {
                    await_index: 0,
                    resume: AsyncStateId(2),
                    cancel: AsyncStateId(4),
                },
            },
            AsyncState {
                id: AsyncStateId(2),
                transition: AsyncTransition::Resume {
                    next: AsyncStateId(3),
                    panic: AsyncStateId(4),
                },
            },
            AsyncState {
                id: AsyncStateId(3),
                transition: AsyncTransition::Complete,
            },
            AsyncState {
                id: AsyncStateId(4),
                transition: AsyncTransition::Cancel {
                    drop_locals: vec![local],
                },
            },
        ],
    }
}

#[tokio::test]
async fn deterministic_and_tokio_schedulers_have_identical_event_order() {
    let parent = ParentScopeId(7);
    let expected = vec![
        TaskEvent::Started { parent },
        TaskEvent::Suspended { await_index: 0 },
        TaskEvent::Woken { await_index: 0 },
        TaskEvent::Resumed { await_index: 0 },
        TaskEvent::Completed,
    ];
    assert_eq!(
        run_deterministic_task(&machine(), parent, TaskControl::Complete).unwrap(),
        expected
    );
    assert_eq!(
        run_tokio_task(machine(), parent, TaskControl::Complete)
            .await
            .unwrap(),
        expected
    );
}

#[test]
fn cancellation_uses_the_single_drop_path_exactly_once() {
    let local = DefId {
        module: 0,
        index: 9,
    };
    assert_eq!(
        run_deterministic_task(
            &machine(),
            ParentScopeId(1),
            TaskControl::CancelAt { await_index: 0 }
        )
        .unwrap(),
        [
            TaskEvent::Started {
                parent: ParentScopeId(1)
            },
            TaskEvent::Suspended { await_index: 0 },
            TaskEvent::Cancelled,
            TaskEvent::Dropped {
                locals: vec![local]
            },
        ]
    );
}

#[test]
fn panic_propagates_only_after_live_locals_are_dropped() {
    let failure = run_deterministic_task(
        &machine(),
        ParentScopeId(1),
        TaskControl::PanicAt { await_index: 0 },
    )
    .unwrap_err();
    assert_eq!(
        failure,
        TaskFailure::Panicked {
            events: vec![
                TaskEvent::Started {
                    parent: ParentScopeId(1),
                },
                TaskEvent::Suspended { await_index: 0 },
                TaskEvent::Woken { await_index: 0 },
                TaskEvent::Resumed { await_index: 0 },
                TaskEvent::Panicked,
                TaskEvent::Dropped {
                    locals: vec![DefId {
                        module: 0,
                        index: 9
                    }],
                },
            ],
        }
    );
}
