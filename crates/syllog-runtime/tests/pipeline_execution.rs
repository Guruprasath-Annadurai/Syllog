//! Structured, bounded, deadline-aware pipeline execution contracts.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use syllog_runtime::{
    CircuitPolicy, JoinPolicy, PipelineEvent, PipelinePlan, ProductionPipelineExecutor,
    RetryPolicy, Stage, StageError,
};
use tokio::sync::Notify;
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
    let executor = ProductionPipelineExecutor::new(4, 2).unwrap();
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
        handle.wait().await,
        Err(StageError::Deadline { .. })
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
    let result = handle.wait().await;
    assert!(matches!(result, Err(StageError::Cancelled)));
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
            .await,
        Err(StageError::CircuitOpen { .. })
    ));
    assert_eq!(failures.load(Ordering::SeqCst), 2);
}
