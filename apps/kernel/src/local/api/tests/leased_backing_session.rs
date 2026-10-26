use super::*;
use crate::app::RemoteLeaseRuntime;

/// MP-08 / MP-09 / MP-10 / MP-11 G09: a client of the owning user reaches a leased agent's hidden backing
/// session on the worker by its id and prompts it there, without the home kernel that holds the lease.
#[test]
fn a_client_cannot_prompt_a_leased_agents_backing_session_on_the_worker() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let harness = LocalRouterTestHarness::with_config(config);
    let leased = harness.with_app_mut(|app| {
        let lease = RemoteLeaseRuntime::new(app)
            .create_execution_lease(
                "home-kernel",
                "session-home",
                "agent-home",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        RemoteLeaseRuntime::new(app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created")
    });

    let attach = |client_id: &str| {
        harness.dispatch_as_user(
            "user-home",
            LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: leased.backing_session_id.clone(),
                client_id: client_id.to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            }),
        )
    };
    let refused = |result: Result<LocalDaemonResponse, DaemonError>| match result {
        Err(DaemonError::LeasedBackingSessionRefused {
            session_id,
            home_session_id,
        }) => {
            assert_eq!(session_id, leased.backing_session_id);
            assert_eq!(home_session_id, "session-home");
        }
        other => panic!("a client reached the leased agent's backing session: {other:?}"),
    };
    refused(attach("direct-client"));
    refused(harness.dispatch_as_user(
        "user-home",
        LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: leased.backing_session_id.clone(),
            attachment_id: "attachment-forged".to_string(),
            target_agent_id: Some(leased.backing_agent_id.clone()),
            prompt: "prompt that bypasses the home kernel\n".to_string(),
            attachments: Vec::new(),
        }),
    ));
}
