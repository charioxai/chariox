//! MP-08 / MP-10 / MP-11: prior task permission at the real shared bridges.
use super::tests::{fixture, fixture_with_options, running};
use super::*;
use crate::session::{NativeInteractionOrigin, RuntimeInteractionKind, RuntimeInteractionLevel};

#[tokio::test]
async fn sudo_operations_projected_worker_uses_home_task() {
    let f = fixture();
    let mut turn = running(&f);
    let mut binding = crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".into(),
        worker_machine_id: "worker-machine".into(),
        execution_lease_id: "worker-lease".into(),
        leased_agent_id: "leased-agent".into(),
        active_worker_provider_run_id: Some("worker-run".into()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    f.app
        .lock()
        .await
        .agents_mut()
        .bind_remote_execution(&turn.agent_id, binding.clone())
        .unwrap();
    turn.placement = Some(binding.execution_lease_id.clone());
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .insert(turn.entry_id.clone(), turn.clone());
    assert!(
        f.state
            .sudo_task_authorizes_agent(&turn.session_id, &turn.agent_id)
            .await,
        "the home task must resolve the projected worker run"
    );
    binding.execution_lease_id = "replacement-lease".into();
    f.app
        .lock()
        .await
        .agents_mut()
        .bind_remote_execution(&turn.agent_id, binding)
        .unwrap();
    assert!(
        !f.state
            .sudo_task_authorizes_agent(&turn.session_id, &turn.agent_id)
            .await,
        "a replacement lease must not inherit task authority"
    );
}

#[tokio::test]
async fn sudo_operations_forwarded_worker_permissions_use_projected_home_task() {
    let f = fixture();
    let mut turn = running(&f);
    let binding = crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".into(),
        worker_machine_id: "worker-machine".into(),
        execution_lease_id: "worker-lease".into(),
        leased_agent_id: "leased-agent".into(),
        active_worker_provider_run_id: Some("worker-run".into()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    f.app
        .lock()
        .await
        .agents_mut()
        .bind_remote_execution(&turn.agent_id, binding.clone())
        .unwrap();
    turn.placement = Some(binding.execution_lease_id.clone());
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .insert(turn.entry_id.clone(), turn.clone());
    let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
        home_session_id: turn.session_id.clone(),
        home_agent_id: turn.agent_id.clone(),
        leased_agent_id: binding.leased_agent_id.clone(),
        worker_provider_run_id: "worker-run".into(),
        home_prompt_id: turn.prompt_id.clone(),
    };
    let make = |id: &str, origin: NativeInteractionOrigin| {
        RuntimeInteraction::new(
            id,
            &binding.leased_agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Execute authorized task operation".into()),
            "permission-only gate",
            vec![
                RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(300),
            Some("deny".into()),
        )
        .with_native_origin(Some(origin))
    };
    for interaction in [
        make(
            "watched-process-forwarded",
            NativeInteractionOrigin::Prompt {
                provider_run_id: "worker-run".into(),
                prompt_id: "worker-prompt".into(),
            },
        ),
        make(
            "claude-permission-forwarded",
            NativeInteractionOrigin::Prompt {
                provider_run_id: "worker-run".into(),
                prompt_id: "worker-prompt".into(),
            },
        ),
        make(
            "claude-workspace-trust-forwarded",
            NativeInteractionOrigin::ProviderStartup {
                provider_run_id: "worker-run".into(),
            },
        ),
    ] {
        let id = interaction.id().to_string();
        let answer = tokio::time::timeout(
            Duration::from_millis(500),
            crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
                &f.state,
                context.clone(),
                interaction,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{id}: forwarded permission asked again"))
        .unwrap();
        assert_eq!(answer.choice_id.as_deref(), Some("allow_once"), "{id}");
    }
    let mut stale = context.clone();
    stale.home_prompt_id = Some("other-home-prompt".into());
    let refused = tokio::time::timeout(
        Duration::from_millis(500),
        crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
            &f.state,
            stale,
            make(
                "watched-process-forwarded-stale",
                NativeInteractionOrigin::Prompt {
                    provider_run_id: "worker-run".into(),
                    prompt_id: "worker-prompt".into(),
                },
            ),
        ),
    )
    .await;
    assert!(
        !matches!(&refused, Ok(Ok(answer)) if answer.choice_id.as_deref() == Some("allow_once")),
        "another home prompt must not reuse the task consent: {refused:?}"
    );
}

#[tokio::test]
async fn sudo_operations_native_permissions_reuse_task_consent() {
    let f = fixture();
    let turn = running(&f);
    let mut failures = Vec::new();
    for prefix in [
        "codex-command-approval",
        "codex-file-change-approval",
        "codex-exec-command-approval",
        "codex-apply-patch-approval",
        "opencode-permission",
        "claude-permission",
        "claude-native-permission",
        "claude-rendered-permission",
        "watched-process",
    ] {
        let interaction = RuntimeInteraction::new(
            format!("{prefix}-covered"),
            &turn.agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Execute authorized task operation".into()),
            "permission-only gate",
            vec![
                RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(300),
            Some("deny".into()),
        )
        .with_native_origin(Some(NativeInteractionOrigin::Prompt {
            provider_run_id: f.run.id().into(),
            prompt_id: turn.prompt_id.clone().unwrap(),
        }));
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            crate::runtime::native_interaction_bridge::request_provider_native_interaction(
                &f.state,
                &turn.session_id,
                interaction,
                "sudo_operations_test",
            ),
        )
        .await;
        if !result.is_ok_and(|reply| {
            reply.is_ok_and(|answer| answer.choice_id.as_deref() == Some("allow_once"))
        }) {
            failures.push(prefix);
        }
    }
    assert!(
        failures.is_empty(),
        "covered native permissions asked again: {failures:?}"
    );
    assert!(f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn sudo_operations_host_workspace_parameters_reuse_task_consent() {
    let f = fixture();
    let turn = running(&f);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    for limit in [1, 2, 3] {
        let request = LocalDaemonRequest::ListWorkspaceFiles(ListWorkspaceFilesRequest {
            workspace_id: session.workspace_id().into(),
            worktree_id: session.worktree_id().into(),
            path_prefix: None,
            compare_ref: None,
            limit: Some(limit),
        });
        assert!(
            f.state
                .authorize_sudo_request(&turn.entry_id, &request)
                .is_ok(),
            "covered workspace query requested new scope"
        );
    }
    let foreign = LocalDaemonRequest::ListWorkspaceFiles(ListWorkspaceFilesRequest {
        workspace_id: session.workspace_id().into(),
        worktree_id: "foreign".into(),
        path_prefix: None,
        compare_ref: None,
        limit: None,
    });
    assert!(f
        .state
        .authorize_sudo_request(&turn.entry_id, &foreign)
        .is_err());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn sudo_operations_required_watched_command_runs() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let f = fixture_with_options(None, true);
    let turn = running(&f);
    f.state
        .owned
        .provider_store
        .update_run_execution_config(
            f.run.id(),
            crate::provider::AgentExecutionMode::Build,
            crate::provider::AgentPermissionLevel::Required,
        )
        .unwrap();
    let Outcome::Task(task) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: turn.owner_user_id.clone(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt_id.clone().unwrap(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("task missing")
    };
    let result = tokio::time::timeout(Duration::from_secs(5), f.router.dispatch_authenticated_runtime_tool_call(
        "sudo-fixture-bearer", "chariox.events.process", serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id, "label":"covered watched command", "argv":["/bin/echo","sudo-watched-done"], "match_text":"sudo-watched-done"})
    )).await.expect("covered watched command asked again").expect("covered watched command refused");
    assert!(result.ok, "{result:?}");
    f.state.owned.agent_wakes.processes.shutdown();
}

#[tokio::test]
async fn sudo_operations_startup_trust_requires_approved_workspace_launch() {
    let f = fixture();
    let turn = running(&f);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &turn.agent_id)
        .unwrap();
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&turn.entry_id)
        .unwrap()
        .prompt_id = None;
    assert!(
        f.state.sudo_for_provider_run(f.run.id()).is_err(),
        "approved startup is not a live tool turn"
    );
    let interaction = RuntimeInteraction::new(
        "claude-workspace-trust-covered",
        &turn.agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Trust workspace?".into()),
        "Trust this workspace",
        vec![
            RuntimeInteractionChoice::new("allow_once", "Trust workspace", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        Some(300),
        Some("deny".into()),
    )
    .with_native_origin(Some(NativeInteractionOrigin::ProviderStartup {
        provider_run_id: f.run.id().into(),
    }));
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        crate::runtime::native_interaction_bridge::request_provider_native_interaction(
            &f.state,
            &turn.session_id,
            interaction,
            "sudo_operations_test",
        ),
    )
    .await
    .expect("approved workspace launch asked again")
    .unwrap();
    assert_eq!(result.choice_id.as_deref(), Some("allow_once"));
    assert!(
        f.state.sudo_for_provider_run(f.run.id()).is_err(),
        "trust must not activate sudo tools"
    );
}

#[tokio::test]
async fn sudo_operations_live_sync_mutation_reuses_task_consent() {
    let f = fixture();
    let turn = running(&f);
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        f.state.maybe_gate_workspace_live_sync_mutation(
            &turn.session_id,
            Some(&turn.agent_id),
            crate::provider::AgentPermissionLevel::Required,
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL,
            &serde_json::json!({"path":"authorized.txt", "content_text":"covered task"}),
        ),
    )
    .await
    .expect("covered workspace mutation asked again")
    .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn sudo_operations_workspace_extension_reuses_task_consent() {
    let f = fixture();
    let turn = running(&f);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let mut agent = f.state.owned.agent_store.get_agent(&turn.agent_id).unwrap();
    agent.set_permission_level_override(Some(crate::provider::AgentPermissionLevel::Required));
    let source = f.run.working_directory().unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        f.state.maybe_gate_extension_registration(
            &session,
            &agent,
            "skill",
            Some("task-skill"),
            Some(source),
        ),
    )
    .await
    .expect("covered workspace extension asked again")
    .unwrap();
    assert!(result.is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn sudo_operations_claude_watched_command_reuses_task_consent() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let f = super::tests::fixture_with_run_profile(None, true, "claude", "claude");
    let turn = running(&f);
    let Outcome::Task(task) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: turn.owner_user_id.clone(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt_id.clone().unwrap(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("task missing")
    };
    let result = tokio::time::timeout(Duration::from_secs(5), f.router.dispatch_authenticated_runtime_tool_call(
        "sudo-fixture-bearer", "chariox.events.process", serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id, "label":"covered watched command", "argv":["/bin/echo","sudo-watched-done"], "match_text":"sudo-watched-done"})
    )).await.expect("covered watched command asked again").expect("covered watched command refused");
    assert!(result.ok, "{result:?}");
    f.state.owned.agent_wakes.processes.shutdown();
}

#[tokio::test]
async fn sudo_operations_never_authorize_new_origin_or_arbitrary_approval() {
    let f = fixture();
    let turn = running(&f);
    let make = |id: &str, kind, prompt: &str, passkey: bool| {
        let choice = RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None);
        let choice = if passkey {
            choice.requiring_passkey()
        } else {
            choice
        };
        RuntimeInteraction::new(
            id,
            &turn.agent_id,
            kind,
            RuntimeInteractionLevel::Warning,
            Some("Permission".into()),
            "control",
            vec![choice],
            None,
            Some(300),
            None,
        )
        .with_native_origin(Some(NativeInteractionOrigin::Prompt {
            provider_run_id: f.run.id().into(),
            prompt_id: prompt.into(),
        }))
    };
    for interaction in [
        make(
            "codex-command-approval-stale",
            RuntimeInteractionKind::Permission,
            "different-prompt",
            false,
        ),
        make(
            "codex-command-approval-question",
            RuntimeInteractionKind::Choice,
            turn.prompt_id.as_deref().unwrap(),
            false,
        ),
        make(
            "codex-command-approval-passkey",
            RuntimeInteractionKind::Permission,
            turn.prompt_id.as_deref().unwrap(),
            true,
        ),
        make(
            "app-action-approval-new",
            RuntimeInteractionKind::Permission,
            turn.prompt_id.as_deref().unwrap(),
            false,
        ),
    ] {
        assert!(f
            .state
            .sudo_native_permission_resolution(&turn.session_id, &interaction, None)
            .is_none());
    }
    let interaction = make(
        "codex-command-approval-ended",
        RuntimeInteractionKind::Permission,
        turn.prompt_id.as_deref().unwrap(),
        false,
    );
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .remove(&turn.entry_id);
    assert!(f
        .state
        .sudo_native_permission_resolution(&turn.session_id, &interaction, None)
        .is_none());
}

#[tokio::test]
async fn sudo_operations_new_extension_source_and_no_sudo_write_still_ask() {
    let f = fixture();
    let turn = running(&f);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let mut agent = f.state.owned.agent_store.get_agent(&turn.agent_id).unwrap();
    agent.set_permission_level_override(Some(crate::provider::AgentPermissionLevel::Required));
    let outside = std::env::temp_dir();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        f.state.maybe_gate_extension_registration(
            &session,
            &agent,
            "skill",
            Some("new-source"),
            Some(&outside),
        ),
    )
    .await;
    assert!(result.is_err(), "another source root must ask");
    // Retire that fixture before exercising a different approval subject.
    drop(f);
    let f = fixture();
    let turn = running(&f);
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .remove(&turn.entry_id);
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        f.state.maybe_gate_workspace_live_sync_mutation(
            &turn.session_id,
            Some(&turn.agent_id),
            crate::provider::AgentPermissionLevel::Required,
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL,
            &serde_json::json!({"path":"unapproved.txt","content_text":"control"}),
        ),
    )
    .await;
    assert!(result.is_err(), "no sudo write must ask");
}

#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn sudo_operations_watched_command_refused_without_linux_process_groups() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let f = fixture_with_options(None, true);
    let turn = running(&f);
    let Outcome::Task(task) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: turn.owner_user_id.clone(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt_id.clone().unwrap(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("task missing")
    };
    let result = tokio::time::timeout(Duration::from_secs(5), f.router.dispatch_authenticated_runtime_tool_call(
        "sudo-fixture-bearer", "chariox.events.process", serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id, "label":"covered watched command", "argv":["/bin/echo","sudo-watched-done"], "match_text":"sudo-watched-done"})
    )).await.expect("watched command refusal timed out");
    let message = match result {
        Ok(result) => {
            assert!(!result.ok, "{result:?}");
            format!("{result:?}")
        }
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains("Linux process-group ownership"),
        "{message}"
    );
}
