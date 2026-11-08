//! MP-08 / MP-09 / MP-10 / MP-11: the enrolled owner's terminal controls the tasks and goals of
//! its room made before enrollment under `local`; anyone else's terminal still cannot.
use super::*;

struct Room {
    harness: crate::local::test_support::LocalRouterTestHarness,
    state: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    attachment_id: String,
}

fn enrolled_owner_room(label: &str) -> Room {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    let harness = crate::local::test_support::LocalRouterTestHarness::with_config(config);
    let request = harness
        .fixture_session_request(label)
        .with_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
    let (session, agent) = harness.with_app_mut(|app| {
        crate::app::KernelSessionService::new(app)
            .create_session(request)
            .unwrap()
    });
    let state = harness.runtime_state();
    let attachment = state
        .owned
        .attachment_store
        .attach(
            &mut state.owned.session_store.write(),
            crate::attachment::AttachRequest::new(
                session.id(),
                "owner-terminal",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ),
        )
        .unwrap();
    assert_eq!(
        attachment.owner_user_id(),
        crate::session::DEFAULT_LOCAL_USER_ID
    );
    Room {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
        attachment_id: attachment.id().into(),
        state,
        harness,
    }
}

#[test]
fn the_enrolled_owner_terminal_cancels_a_task_of_a_room_made_before_enrollment() {
    let room = enrolled_owner_room("enrolled-owner-cancel");
    let refused = |user: &str| {
        let request = crate::local::CancelAgentTaskRequest {
            session_id: room.session_id.clone(),
            attachment_id: room.attachment_id.clone(),
            task_id: "missing-task".into(),
            revision: 1,
        };
        let mut command = crate::runtime::command::KernelCommand::from_local_request(
            "enrolled-owner-cancel",
            None,
            None,
            &LocalDaemonRequest::CancelAgentTask(request.clone()),
        );
        command.caller.user_id = Some(user.into());
        command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
        room.harness
            .block_on_test_task(room.state.cancel_owner_agent_task(&command, request))
            .err()
            .is_some_and(|error| error.to_string().contains("only the room owner terminal"))
    };
    assert!(!refused("kernel-owner"));
    assert!(refused("guest"));
}

#[test]
fn the_enrolled_owner_terminal_controls_a_goal_of_a_room_made_before_enrollment() {
    let room = enrolled_owner_room("enrolled-owner-goal");
    let refused = |user: &str| {
        let request = crate::local::SubmitPromptRequest {
            session_id: room.session_id.clone(),
            attachment_id: room.attachment_id.clone(),
            target_agent_id: Some(room.agent_id.clone()),
            prompt: "/goal pause missing-goal 1".into(),
            attachments: vec![],
        };
        room.harness
            .block_on_test_task(room.state.control_goal_prompt(&request, user))
            .err()
            .is_some_and(|error| error.to_string().contains("only the room owner controls"))
    };
    assert!(!refused("kernel-owner"));
    assert!(refused("guest"));
}

#[test]
fn the_enrolled_owner_grants_and_revokes_extensions_of_an_agent_made_before_enrollment() {
    let room = enrolled_owner_room("enrolled-owner-extension");
    let refused = |user: &str| {
        let request = crate::local::RevokeAgentExtensionRequest {
            agent_ref: room.agent_id.clone(),
            kind: crate::local::ExtensionKind::Script,
            name: "missing-script".into(),
        };
        let result = room.harness.block_on_test_task(
            crate::runtime::agent_control_executor::execute_revoke_agent_extension_request(
                &room.state,
                user,
                request,
            ),
        );
        matches!(result, Err(DaemonError::OwnershipAccessDenied { .. }))
    };
    assert!(!refused("kernel-owner"));
    assert!(refused("guest"));
}

#[test]
fn uninstalling_an_app_of_the_enrolled_owner_invalidates_copies_of_a_pre_enrollment_agent() {
    let room = enrolled_owner_room("enrolled-owner-uninstall");
    let (workflow_id, endpoint_id, node_id) = {
        let mut sessions = room.state.owned.session_store.write();
        let workflow = sessions
            .create_workflow(&room.session_id, Some("uninstall-copies".into()))
            .unwrap();
        let node = sessions
            .add_workflow_node_owned(
                &room.session_id,
                workflow.id(),
                &room.agent_id,
                crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
                crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
                "Uninstall copies node".to_string(),
            )
            .unwrap();
        let endpoint = sessions
            .create_workflow_endpoint(
                &room.session_id,
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .unwrap();
        (
            workflow.id().to_string(),
            endpoint.id().to_string(),
            node.id().to_string(),
        )
    };
    let source = room
        .state
        .owned
        .agent_store
        .get_agent(&room.agent_id)
        .unwrap();
    let revision = room
        .state
        .owned
        .session_store
        .get_session(&room.session_id)
        .unwrap()
        .workflow(&workflow_id)
        .unwrap()
        .revision();
    let clone = room
        .state
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source, &room.session_id, "worktree-2");
    {
        let mut sessions = room.state.owned.session_store.write();
        for (id, primary, agent_id) in [
            ("instance-primary", true, room.agent_id.clone()),
            ("instance-clone", false, clone.id().to_string()),
        ] {
            sessions
                .register_workflow_runtime_instance(
                    &room.session_id,
                    crate::session::WorkflowEndpointRuntimeInstance::new(
                        id,
                        &workflow_id,
                        &endpoint_id,
                        revision,
                        if primary { 1 } else { 2 },
                        primary,
                        std::collections::BTreeMap::from([(node_id.clone(), agent_id)]),
                        "worktree-1",
                    ),
                )
                .unwrap();
        }
    }
    room.state
        .owned
        .agent_store
        .grant_extension(
            &room.agent_id,
            crate::extension::ExtensionGrant::app("installed"),
        )
        .unwrap();

    let revision_now = || {
        room.state
            .owned
            .session_store
            .get_session(&room.session_id)
            .unwrap()
            .workflow(&workflow_id)
            .unwrap()
            .revision()
    };

    room.harness.block_on_test_task(
        room.state
            .refresh_uninstalled_app_bindings("guest", "installed"),
    );
    assert_eq!(revision_now(), revision);

    room.harness.block_on_test_task(
        room.state
            .refresh_uninstalled_app_bindings("kernel-owner", "installed"),
    );
    assert_eq!(revision_now(), revision + 1);
}
