//! MP-08 / MP-10 / MP-11: actual preparation/expected-head regression, supplementary fixture.
use super::*;

#[tokio::test]
async fn cancelling_preparing_head_promotes_the_surviving_prompt_without_new_submission() {
    let (root, app, mut runtime, session, agent, run) = compact_fixture().await;
    let account = runtime
        .owned
        .agent_store
        .get_agent(&agent)
        .unwrap()
        .provider_account_profile()
        .to_string();
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &account,
    )
    .unwrap();
    let attachment = runtime
        .with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &session,
                    "preparation-cancel-observer",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )
        })
        .await
        .unwrap();
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    for (id, text) in [
        ("head-a", "cancel this preparing request"),
        ("head-b", "surviving queued request"),
    ] {
        runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(
                &snapshot,
                crate::session::PromptQueueItem::new(
                    id,
                    attachment.id(),
                    &agent,
                    text,
                    crate::session::PromptStatus::Queued,
                ),
                true,
            )
            .unwrap();
    }
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&snapshot, &agent);
    let head_id = queued
        .front()
        .expect("preparation queue head")
        .id()
        .to_string();
    runtime
        .owned
        .mirror_prompt_owner_agent_state(&session, &agent, active, queued)
        .unwrap();
    let probe = Arc::new(tokio::sync::Notify::new());
    runtime.observe_app_lock_wait_for_test(probe.clone());
    let guard = app.lock().await;
    let promoter = runtime.clone();
    let promoter_session = session.clone();
    let promoter_agent = agent.clone();
    let task = tokio::spawn(async move {
        promoter
            .advance_project_queued_prompt_after_settlement(&promoter_session, &promoter_agent)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), probe.notified())
        .await
        .unwrap();
    runtime
        .owned
        .cancel_queued_prompt(&session, &agent, attachment.id(), &head_id)
        .unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    drop(guard);
    let promoted = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let submitted = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !root.path().join("surviving-submitted").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
    assert!(
        promoted.is_some_and(|p| p.prompt() == "surviving queued request"),
        "cancelled preparation head stranded the surviving queued request"
    );
    assert!(
        submitted,
        "surviving queued request did not reach the actual provider fixture input"
    );
}
