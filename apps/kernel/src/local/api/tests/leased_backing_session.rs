use super::*;
use crate::app::RemoteLeaseRuntime;

fn harness_with_leased_agent() -> (LocalRouterTestHarness, crate::execution_lease::LeasedAgent) {
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
    (harness, leased)
}

/// MP-08 / MP-09 / MP-10 / MP-11 G09: a client of the owning user reaches a leased agent's hidden backing
/// session on the worker by its id and prompts it there, without the home kernel that holds the lease.
#[test]
fn a_client_cannot_prompt_a_leased_agents_backing_session_on_the_worker() {
    let (harness, leased) = harness_with_leased_agent();

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

/// MP-08 / MP-09 / MP-10 / MP-11 G09: a local terminal on a kernel without a Cloud profile carries no
/// user, so it skips room membership; it must still be refused on the backing session.
#[test]
fn a_profile_less_local_client_cannot_reach_a_leased_agents_backing_session() {
    let (harness, leased) = harness_with_leased_agent();
    let refused = |result: Result<LocalDaemonResponse, DaemonError>| match result {
        Err(DaemonError::LeasedBackingSessionRefused {
            session_id,
            home_session_id,
        }) => {
            assert_eq!(session_id, leased.backing_session_id);
            assert_eq!(home_session_id, "session-home");
        }
        other => panic!("a local client reached the leased agent's backing session: {other:?}"),
    };
    refused(harness.dispatch(LocalDaemonRequest::AttachToSession(
        AttachToSessionRequest {
            session_id: leased.backing_session_id.clone(),
            client_id: "local-terminal".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        },
    )));
    refused(
        harness.dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: leased.backing_session_id.clone(),
            attachment_id: "attachment-forged".to_string(),
            target_agent_id: Some(leased.backing_agent_id.clone()),
            prompt: "prompt that bypasses the home kernel\n".to_string(),
            attachments: Vec::new(),
        })),
    );
}
