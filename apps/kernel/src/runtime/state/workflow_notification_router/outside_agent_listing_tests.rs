//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17, ON-18): a grant lists the sources it may follow, through
//! the real router, and everything listed can be followed. Supplementary regressions, not live
//! acceptance.
use super::outside_agent_tests::{dispatch, register, restore};
use super::tests::router;
use super::*;
use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};

async fn listed(
    router: &crate::runtime::router::CommandRouter,
    grant: &str,
) -> Vec<WorkflowNotificationSourceSummary> {
    match dispatch(
        router,
        grant,
        serde_json::json!({"ListNotificationSources":{}}),
    )
    .await
    .unwrap()
    {
        LocalDaemonResponse::NotificationSources { sources } => sources,
        other => panic!("expected notification sources, got {other:?}"),
    }
}

#[tokio::test]
async fn a_grant_lists_the_sources_it_can_follow_and_follows_every_one_listed() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let (b, _, _) = f.workflow("disabled");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let grant = kernel.insert_access_grant_for_test(&f.session);
    // The owner-action source does not exist yet: nothing has blocked and no Room listed.
    assert!(kernel
        .owned
        .durable_state_store
        .notification_inventory("local")
        .unwrap()
        .0
        .is_empty());
    let own = register(&kernel, &f, &a);
    // A disabled source and another owner's source are not offered.
    kernel
        .execute_workflow_notification_request(
            LocalDaemonRequest::RegisterWorkflowNotificationSource(
                RegisterWorkflowNotificationSourceRequest {
                    session_id: f.session.clone(),
                    workflow_ref: b,
                    enabled: false,
                    exit_node_id: "exit-1".into(),
                },
            ),
            "local",
        )
        .unwrap();
    let mut stranger = own.clone();
    stranger.source_id = "wf_source_stranger".into();
    stranger.owner_user_id = "stranger".into();
    kernel
        .owned
        .durable_state_store
        .notify(NotificationOperation::Register(SourceAdmission {
            source: stranger,
            workflow_json: serde_json::to_string(
                &f.sessions.resolve_workflow_ref(&f.session, &a).unwrap(),
            )
            .unwrap(),
        }))
        .unwrap();
    restore(&kernel, &f);

    let sources = listed(&router, &grant).await;
    let mut names: Vec<_> = sources
        .iter()
        .map(|s| (s.kind, s.display_name.as_str()))
        .collect();
    names.sort_by_key(|(_, name)| name.to_string());
    assert_eq!(
        names,
        [
            (
                NotificationSourceKind::OwnerAction,
                "Agent needs your action"
            ),
            (NotificationSourceKind::WorkflowExit, "reviewer"),
        ],
        "{sources:?}"
    );
    assert!(sources.iter().all(|s| s.available));
    assert!(sources.iter().any(|s| s.source_id == own.source_id));
    // Listing created the owner-action source, so it can be followed before any block.
    let ids: Vec<_> = sources.iter().map(|s| s.source_id.clone()).collect();
    let LocalDaemonResponse::EventSubscription { subscription } = dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":ids}}),
    )
    .await
    .unwrap() else {
        panic!("expected a subscription")
    };
    assert_eq!(subscription.sources.len(), 2);
    // A second listing changes nothing.
    assert_eq!(listed(&router, &grant).await.len(), 2);
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn only_an_access_grant_lists_notification_sources() {
    let mut f = Fixture::new();
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let request: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"ListNotificationSources":{}})).unwrap();
    // A terminal-class caller is refused with the same sentence as the other event requests.
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "terminal-sources",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(KernelConnectionClass::Terminal);
    assert!(router.dispatch(command, request.clone()).await.is_err());
    // A reach caller (a Room agent's run) never lists them either.
    let reach = crate::runtime::kernel_gateway::Origin {
        grant_id: "reach:run-1".into(),
        harness: "test".into(),
    };
    assert!(router
        .kernel_gateway()
        .event_request(&reach, &request, None)
        .await
        .is_err());
    // A revoked grant lists nothing.
    let grant = kernel.insert_access_grant_for_test(&f.session);
    kernel
        .revoke_kernel_access(Some("local"), Some(&grant), "owner")
        .unwrap();
    assert!(dispatch(
        &router,
        &grant,
        serde_json::json!({"ListNotificationSources":{}})
    )
    .await
    .is_err());
    drop((router, kernel));
    cleanup(f);
}
