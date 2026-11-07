//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17): a subscribe request is admitted by the grant's inbox
//! before any follow commits, and a refusal leaves no follow behind. Supplementary regressions.
use super::outside_agent_tests::{dispatch, refusal, register};
use super::tests::router;
use super::*;
use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};

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

fn local_session(f: &Fixture) -> serde_json::Value {
    serde_json::json!({"kernel":"local","session_id":f.session})
}

#[tokio::test]
async fn following_a_source_on_an_ended_inbox_is_refused_and_follows_nothing() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);
    let follow = serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}});
    dispatch(&router, &grant, follow.clone()).await.unwrap();
    dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"all":true}}),
    )
    .await
    .unwrap();
    assert!(followed(&kernel, &grant).is_empty());
    assert!(refusal(dispatch(&router, &grant, follow).await).contains("subscription ended"));
    assert!(
        followed(&kernel, &grant).is_empty(),
        "the refused request left a follow that counts against the grant"
    );
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_mixed_request_on_an_ended_inbox_is_refused_and_leaves_no_follow() {
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
    dispatch(
        &router,
        &grant,
        serde_json::json!({"UnsubscribeEvents":{"all":true}}),
    )
    .await
    .unwrap();
    let mixed = serde_json::json!({"SubscribeEvents":{"targets":[local_session(&f)],"sources":[source.source_id]}});
    assert!(refusal(dispatch(&router, &grant, mixed).await).contains("subscription ended"));
    assert!(followed(&kernel, &grant).is_empty());
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_mixed_request_whose_source_is_refused_adds_no_target() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);
    dispatch(
        &router,
        &grant,
        serde_json::json!({"SubscribeEvents":{"targets":[],"sources":[source.source_id]}}),
    )
    .await
    .unwrap();
    let mixed = serde_json::json!({"SubscribeEvents":{"targets":[local_session(&f)],"sources":["wf_source_unknown"]}});
    assert!(refusal(dispatch(&router, &grant, mixed).await).contains("source not available"));
    let part = kernel
        .outside_store()
        .outside_part(&grant)
        .unwrap()
        .unwrap();
    assert!(
        part.targets.is_empty(),
        "the refused request left its target followed"
    );
    assert_eq!(followed(&kernel, &grant), [source.source_id]);
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn an_unsubscribe_that_is_refused_keeps_the_sources_it_named() {
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
    // The target names a kernel that cannot be resolved, so the request is refused as a whole.
    let mixed = serde_json::json!({"UnsubscribeEvents":{"targets":[{"kernel":"nowhere","session_id":"s"}],"sources":[source.source_id.clone()]}});
    dispatch(&router, &grant, mixed).await.unwrap_err();
    assert_eq!(followed(&kernel, &grant), [source.source_id]);
    drop((router, kernel));
    cleanup(f);
}

#[tokio::test]
async fn a_kernel_that_refuses_the_installation_takes_back_the_whole_request() {
    use crate::runtime::kernel_gateway::{Added, GatewayError};
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let router = router(&mut f);
    let kernel = router.runtime_state();
    let source = register(&kernel, &f, &a);
    let grant = kernel.insert_access_grant_for_test(&f.session);
    let remote = EventTarget {
        kernel: "remote".into(),
        session_id: "remote-session".into(),
        agent_id: None,
    };
    kernel
        .subscribe_outside_part(&grant, true, vec![remote.clone()])
        .unwrap();
    kernel
        .follow_notification_sources(&grant, "local", std::slice::from_ref(&source.source_id))
        .unwrap();
    let refused = GatewayError::Remote {
        name: "remote".into(),
        code: "events_refused".into(),
        message: "refused".into(),
        retryable: false,
    };
    let origin = crate::runtime::kernel_gateway::Origin {
        grant_id: grant.clone(),
        harness: "test".into(),
    };
    let remote_kernel = "remote".to_string();
    router
        .kernel_gateway()
        .settle_installations(
            &origin,
            &Added {
                targets: vec![remote],
                sources: vec![source.source_id],
            },
            vec![(&remote_kernel, Err(refused))],
        )
        .unwrap_err();
    assert!(followed(&kernel, &grant).is_empty());
    let part = kernel
        .outside_store()
        .outside_part(&grant)
        .unwrap()
        .unwrap();
    assert!(part.targets.is_empty());
    drop((router, kernel));
    cleanup(f);
}
