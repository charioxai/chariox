//! MP-08 / MP-10 / MP-11: home attachments see a remote profile reservation before worker transport returns.
use super::*;

#[tokio::test]
async fn remote_profile_preparation_is_visible_to_every_home_attachment() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    let attachments = {
        let mut app = app.lock().await;
        let mut attachments = Vec::new();
        for alias in ["local-home", "remote-home"] {
            attachments.push(
                crate::app::KernelSessionService::new(&mut app)
                    .attach(crate::attachment::AttachRequest::new(
                        &session,
                        alias,
                        crate::attachment::ClientCapabilityLevel::FullTerminal,
                    ))
                    .unwrap(),
            );
        }
        app.agents_mut()
            .bind_remote_execution(
                &agent,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "unavailable-worker".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased-agent".into(),
                    active_worker_provider_run_id: Some("worker-run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: None,
                },
            )
            .unwrap();
        attachments
    };
    let result = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(
        result.is_err(),
        "unavailable worker must preserve the home profile"
    );
    assert!(
        !root.path().join("started").exists(),
        "only the worker may compact its own session"
    );
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
    for attachment in attachments {
        let notices = runtime
            .owned
            .terminal_stream
            .drain_notice_records(&session, attachment.id());
        assert!(
            notices
                .iter()
                .any(|notice| notice.agent_id.as_deref() == Some(agent.as_str())
                    && notice.message == "Preparing the worker's profile change…"),
            "each home attachment must observe the worker profile reservation"
        );
        assert!(
            notices
                .iter()
                .any(|notice| notice.agent_id.as_deref() == Some(agent.as_str())
                    && notice
                        .message
                        .starts_with("The worker's profile change failed; the agent keeps ")),
            "a failed worker profile change must not leave the reservation notice as the last word"
        );
    }
}
