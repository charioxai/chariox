//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17): a follow request is all or nothing, and a disabled source
//! stays in what the grant follows until the grant drops it. Supplementary regressions.
use super::outside_agent_tests::{dispatch, refusal, register};
use super::tests::router;
use super::*;
use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};
use crate::durable_state::workflow_notifications::{NotificationOperation, NotificationOutcome};

/// A follow row of `grant` on a source id the kernel may not know: it only counts toward limits.
fn follow_row(kernel: &KernelRuntimeState, f: &Fixture, grant: &str, source_id: &str) {
    let home = kernel.owned.config_projection.snapshot().daemon_id;
    let outcome = kernel
        .owned
        .durable_state_store
        .notify(NotificationOperation::Granted {
            grant: grant.into(),
            attach: Box::new(NotificationOperation::RemoteAttach {
                subscription: WorkflowNotificationSubscription {
                    subscription_id: format!("wf_subscription_{:032x}", rand::random::<u128>()),
                    source_id: source_id.into(),
                    owner_user_id: "local".into(),
                    created_by_user_id: "local".into(),
                    target_kernel_id: home.clone(),
                    target_kind: WorkflowNotificationTargetKind::OutsideAgent,
                    session_id: f.session.clone(),
                    workflow_id: String::new(),
                    publication_id: grant.into(),
                    endpoint_id: grant.into(),
                    queue_id: "grant".into(),
                    source_available: true,
                    source_kernel_id: home,
                    agent_id: None,
                    trigger_id: None,
                },
            }),
        })
        .unwrap();
    assert!(matches!(outcome, NotificationOutcome::Subscription(_)));
}

fn followed(kernel: &KernelRuntimeState, grant: &str) -> Vec<String> {
    kernel
        .owned
        .durable_state_store
        .notification_inventory("local")
        .unwrap()
        .1
        .into_iter()
        .filter(|sub| sub.endpoint_id == grant)
        .map(|sub| sub.source_id)
        .collect()
}

#[tokio::test]
async fn a_follow_that_passes_the_source_limit_follows_none_of_its_sources() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer-a");
    let (b, _, _) = f.workflow("reviewer-b");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let (first, second) = (register(&kernel, &f, &a), register(&kernel, &f, &b));
    let grant = kernel.insert_access_grant_for_test(&f.session);
    for n in 0..63 {
        follow_row(&kernel, &f, &grant, &format!("wf_source_filler_{n}"));
    }
    assert_eq!(
        refusal(
            dispatch(
                &router,
                &grant,
                serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[first.source_id, second.source_id]}}),
            )
            .await
        ),
        "event source limit"
    );
    assert_eq!(
        followed(&kernel, &grant).len(),
        63,
        "the first source of a refused request must not stay followed"
    );
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_follow_that_passes_a_source_fanout_limit_follows_none_of_its_sources() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer-a");
    let (b, _, _) = f.workflow("reviewer-b");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let (first, second) = (register(&kernel, &f, &a), register(&kernel, &f, &b));
    let grant = kernel.insert_access_grant_for_test(&f.session);
    // Other grants already fill the fanout of the second source.
    for n in 0..32 {
        follow_row(&kernel, &f, &format!("other-grant-{n}"), &second.source_id);
    }
    assert_eq!(
        refusal(
            dispatch(
                &router,
                &grant,
                serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[first.source_id, second.source_id]}}),
            )
            .await
        ),
        "notification fanout limit"
    );
    assert!(
        followed(&kernel, &grant).is_empty(),
        "the first source of a refused request must not stay followed"
    );
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_disabled_source_stays_followed_until_the_grant_unsubscribes_it() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);
    dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id.clone()]}}),
    )
    .await
    .unwrap();
    // The owner disables the source; the grant's follow is still there and shows as unavailable.
    kernel
        .execute_workflow_notification_request(
            LocalDaemonRequest::RegisterWorkflowNotificationSource(
                RegisterWorkflowNotificationSourceRequest {
                    session_id: f.session.clone(),
                    workflow_ref: a.clone(),
                    enabled: false,
                    exit_node_id: "exit-1".into(),
                },
            ),
            "local",
        )
        .unwrap();
    let follows = kernel.followed_notification_sources(&grant).unwrap();
    assert_eq!(follows.len(), 1, "{follows:?}");
    assert_eq!(follows[0].source_id, source.source_id);
    assert!(!follows[0].available);
    // `events unsubscribe --source all` names what the grant reports following.
    dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"sources":[follows[0].source_id.clone()]}}),
    )
    .await
    .unwrap();
    assert!(
        followed(&kernel, &grant).is_empty(),
        "re-enabling the source must not resume a detached follow"
    );
    drop((router, kernel));
    cleanup(f);
}
