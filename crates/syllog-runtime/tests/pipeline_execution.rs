//! Structured, bounded, deadline-aware pipeline execution contracts.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use syllog_runtime::{
    CircuitPolicy, JoinPolicy, PipelineEvent, PipelineGroup, PipelinePlan,
    ProductionPipelineExecutor, RetryPolicy, Stage, StageError,
};
use tokio::sync::{Barrier, Notify};
use tokio::time::{Duration, advance};

#[tokio::test]
async fn serial_stages_transform_in_order_and_events_never_contain_payloads() {
    let first = Stage::new("normalize", |_context, input| async move {
        Ok(format!("{input}-normalized"))
    });
    let second = Stage::new("route", |_context, input| async move {
        Ok(format!("{input}-routed"))
    });
    let plan = PipelinePlan::serial(vec![first, second]);
    let executor = ProductionPipelineExecutor::new(8, 2).unwrap();
    let handle = executor.execute(plan, "private-prompt".into());
    let outcome = handle.wait().await.unwrap();

    assert_eq!(outcome.output, ["private-prompt-normalized-routed"]);
    assert_eq!(
        outcome.events,
        [
            PipelineEvent::Started,
            PipelineEvent::StageAttempt {
                stage: "normalize".into(),
                attempt: 1
            },
            PipelineEvent::StageSucceeded {
                stage: "normalize".into()
            },
            PipelineEvent::StageAttempt {
                stage: "route".into(),
                attempt: 1
            },
            PipelineEvent::StageSucceeded {
                stage: "route".into()
            },
            PipelineEvent::Completed,
        ]
    );
    let rendered = format!("{:?}", outcome.events);
    assert!(!rendered.contains("private-prompt"));
    assert!(!rendered.contains("normalized-routed"));
}

#[tokio::test]
async fn fan_out_is_bounded_and_preserve_order_join_is_deterministic() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut branches = Vec::new();
    for index in 0..5 {
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        branches.push(Stage::new(
            format!("branch-{index}"),
            move |_context, _input| {
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(index.to_string())
                }
            },
        ));
    }
    let plan = PipelinePlan::fan_out(branches, JoinPolicy::PreserveInputOrder);
    let outcome = ProductionPipelineExecutor::new(8, 2)
        .unwrap()
        .execute(plan, "input".into())
        .wait()
        .await
        .unwrap();
    assert_eq!(outcome.output, ["0", "1", "2", "3", "4"]);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn retries_use_paused_time_and_deadline_stops_further_attempts() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let stage = Stage::new("unstable", {
        let attempts = Arc::clone(&attempts);
        move |_context, input| {
            let attempts = Arc::clone(&attempts);
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(StageError::retryable("temporary"))
                } else {
                    Ok(input)
                }
            }
        }
    })
    .with_retry(RetryPolicy {
        max_attempts: 3,
        backoff: Duration::from_secs(1),
    })
    .with_deadline(Duration::from_secs(5));
    let handle = ProductionPipelineExecutor::new(4, 1)
        .unwrap()
        .execute(PipelinePlan::serial(vec![stage]), "ok".into());
    tokio::task::yield_now().await;
    advance(Duration::from_secs(2)).await;
    assert_eq!(handle.wait().await.unwrap().output, ["ok"]);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);

    let never = Stage::new("deadline", |_context, _input| async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok("late".into())
    })
    .with_deadline(Duration::from_secs(3));
    let handle = ProductionPipelineExecutor::new(4, 1)
        .unwrap()
        .execute(PipelinePlan::serial(vec![never]), "input".into());
    tokio::task::yield_now().await;
    advance(Duration::from_secs(3)).await;
    assert!(matches!(
        handle.wait().await.unwrap_err().error,
        StageError::Deadline { .. }
    ));
}

#[tokio::test]
async fn cancellation_waits_for_child_drop_and_circuit_opens_at_threshold() {
    let entered = Arc::new(Notify::new());
    let dropped = Arc::new(Notify::new());
    let stage = Stage::new("child", {
        let entered = Arc::clone(&entered);
        let dropped = Arc::clone(&dropped);
        move |context, _input| {
            let entered = Arc::clone(&entered);
            let dropped = Arc::clone(&dropped);
            async move {
                entered.notify_one();
                context.cancelled().await;
                dropped.notify_one();
                Err(StageError::Cancelled)
            }
        }
    });
    let handle = ProductionPipelineExecutor::new(2, 1)
        .unwrap()
        .execute(PipelinePlan::serial(vec![stage]), "input".into());
    entered.notified().await;
    handle.cancel();
    let failure = handle.wait().await.unwrap_err();
    assert_eq!(failure.error, StageError::Cancelled);
    assert!(failure.events.contains(&PipelineEvent::Cancelled));
    dropped.notified().await;

    let failures = Arc::new(AtomicUsize::new(0));
    let circuit_stage = Stage::new("protected", {
        let failures = Arc::clone(&failures);
        move |_context, _input| {
            let failures = Arc::clone(&failures);
            async move {
                failures.fetch_add(1, Ordering::SeqCst);
                Err(StageError::retryable("down"))
            }
        }
    })
    .with_circuit(CircuitPolicy {
        failure_threshold: 2,
        reset_after: Duration::from_secs(60),
    });
    let executor = ProductionPipelineExecutor::new(2, 1).unwrap();
    for _ in 0..2 {
        assert!(
            executor
                .execute(
                    PipelinePlan::serial(vec![circuit_stage.clone()]),
                    "x".into()
                )
                .wait()
                .await
                .is_err()
        );
    }
    assert!(matches!(
        executor
            .execute(PipelinePlan::serial(vec![circuit_stage]), "x".into())
            .wait()
            .await
            .unwrap_err()
            .error,
        StageError::CircuitOpen { .. }
    ));
    assert_eq!(failures.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn failed_pipelines_retain_events_and_non_cooperative_tasks_are_supervised_after_grace() {
    let failed = Stage::new("failed", |_context, _input| async move {
        Err(StageError::Fatal {
            message: "sanitized".into(),
        })
    });
    let failure = ProductionPipelineExecutor::new(8, 1)
        .unwrap()
        .execute(PipelinePlan::serial(vec![failed]), "secret".into())
        .wait()
        .await
        .unwrap_err();
    assert!(matches!(failure.error, StageError::Fatal { .. }));
    assert_eq!(
        failure.events,
        [
            PipelineEvent::Started,
            PipelineEvent::StageAttempt {
                stage: "failed".into(),
                attempt: 1,
            },
            PipelineEvent::StageFailed {
                stage: "failed".into(),
                kind: syllog_runtime::StageErrorKind::Fatal,
            },
        ]
    );
    assert!(!format!("{:?}", failure.events).contains("secret"));

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let stubborn = Stage::new("stubborn", {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        move |_context, input| {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            async move {
                entered.notify_one();
                release.notified().await;
                Ok(input)
            }
        }
    });
    let executor = ProductionPipelineExecutor::new(8, 1)
        .unwrap()
        .with_cancellation_grace(Duration::from_millis(20));
    let handle = executor.execute(PipelinePlan::serial(vec![stubborn]), "input".into());
    entered.notified().await;
    handle.cancel();
    let failure = handle.wait().await.unwrap_err();
    assert_eq!(failure.error, StageError::Cancelled);
    assert!(failure.events.contains(&PipelineEvent::TaskDetached));
    assert_eq!(executor.supervised_detached_tasks(), 1);
    release.notify_waiters();
    for _ in 0..10 {
        if executor.supervised_detached_tasks() == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(executor.supervised_detached_tasks(), 0);
}

#[tokio::test]
async fn structured_child_scope_cancels_and_joins_siblings_before_failure_returns() {
    let barrier = Arc::new(Barrier::new(2));
    let cleaned = Arc::new(Notify::new());
    let failing = Stage::new("failing-child", {
        let barrier = Arc::clone(&barrier);
        move |_context, _input| {
            let barrier = Arc::clone(&barrier);
            async move {
                barrier.wait().await;
                Err(StageError::Fatal {
                    message: "failed".into(),
                })
            }
        }
    });
    let sibling = Stage::new("sibling", {
        let barrier = Arc::clone(&barrier);
        let cleaned = Arc::clone(&cleaned);
        move |context, _input| {
            let barrier = Arc::clone(&barrier);
            let cleaned = Arc::clone(&cleaned);
            async move {
                barrier.wait().await;
                context.cancelled().await;
                cleaned.notify_one();
                Err(StageError::Cancelled)
            }
        }
    });
    let plan = PipelinePlan::structured(vec![PipelineGroup::FanOut(
        vec![failing, sibling],
        JoinPolicy::PreserveInputOrder,
    )]);
    let failure = ProductionPipelineExecutor::new(16, 2)
        .unwrap()
        .execute(plan, "input".into())
        .wait()
        .await
        .unwrap_err();
    assert!(matches!(failure.error, StageError::Fatal { .. }));
    cleaned.notified().await;
    assert!(failure.events.iter().any(|event| matches!(
        event,
        PipelineEvent::StageFailed {
            stage,
            kind: syllog_runtime::StageErrorKind::Cancelled
        } if stage == "sibling"
    )));
}
