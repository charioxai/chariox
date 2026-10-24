//! MP-08 / MP-10 / MP-11: prior task permission at the real shared bridges.
use super::tests::{fixture, fixture_with_options, running};
use super::*;
use crate::session::{NativeInteractionOrigin, RuntimeInteractionKind, RuntimeInteractionLevel};

#[tokio::test]
async fn sudo_operations_forwarded_permissions_use_projected_home_run() {
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
    for prefix in [
        "claude-native-permission",
        "watched-process",
        "codex-command-approval",
        "claude-workspace-trust",
    ] {
        let interaction = RuntimeInteraction::new(
            format!("{prefix}-forwarded"),
            &turn.agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Covered worker tool".into()),
            "same task",
            vec![
                RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(1),
            Some("deny".into()),
        )
        .with_native_origin(Some(if prefix == "claude-workspace-trust" {
            NativeInteractionOrigin::ProviderStartup {
                provider_run_id: "worker-run".into(),
            }
        } else {
            NativeInteractionOrigin::Prompt {
                provider_run_id: "worker-run".into(),
                prompt_id: "worker-prompt".into(),
            }
        }));
        let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
            home_session_id: turn.session_id.clone(),
            home_agent_id: turn.agent_id.clone(),
            leased_agent_id: binding.leased_agent_id.clone(),
            worker_provider_run_id: "worker-run".into(),
            home_prompt_id: turn.prompt_id.clone(),
        };
        let answer = tokio::time::timeout(
            Duration::from_secs(2),
            crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
                &f.state,
                context,
                interaction,
            ),
        )
        .await
        .expect("covered worker permission waited for a person")
        .unwrap();
        assert_eq!(answer.choice_id.as_deref(), Some("allow_once"), "{prefix}");
    }
    for stale in ["prompt", "lease"] {
        let mut current_binding = binding.clone();
        if stale == "lease" {
            current_binding.execution_lease_id = "replacement-lease".into();
            f.app
                .lock()
                .await
                .agents_mut()
                .bind_remote_execution(&turn.agent_id, current_binding)
                .unwrap();
        }
        let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
            home_session_id: turn.session_id.clone(),
            home_agent_id: turn.agent_id.clone(),
            leased_agent_id: binding.leased_agent_id.clone(),
            worker_provider_run_id: "worker-run".into(),
            home_prompt_id: Some(if stale == "prompt" {
                "replacement-prompt".into()
            } else {
                turn.prompt_id.clone().unwrap()
            }),
        };
        let interaction = RuntimeInteraction::new(
            format!("claude-native-permission-stale-{stale}"),
            &turn.agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Stale worker permission".into()),
            "must not reuse the task",
            vec![
                RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(1),
            Some("deny".into()),
        )
        .with_native_origin(Some(NativeInteractionOrigin::Prompt {
            provider_run_id: "worker-run".into(),
            prompt_id: "worker-prompt".into(),
        }));
        let answer = tokio::time::timeout(
            Duration::from_secs(2),
            crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
                &f.state,
                context,
                interaction,
            ),
        )
        .await
        .expect("stale forwarded permission did not settle");
        assert!(
            !answer.is_ok_and(|a| a.choice_id.as_deref() == Some("allow_once")),
            "stale {stale} inherited consent"
        );
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn sudo_operations_watched_commands_keep_plan_and_write_fences() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    for (mode, write_access) in [
        (
            AgentExecutionMode::Plan,
            crate::provider::ProviderWriteAccessMode::Unrestricted,
        ),
        (
            AgentExecutionMode::Build,
            crate::provider::ProviderWriteAccessMode::WorkspaceLiveSyncManaged,
        ),
        (
            AgentExecutionMode::Build,
            crate::provider::ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
        ),
    ] {
        let f = fixture_with_options(None, true);
        let turn = running(&f);
        let mut snapshot = serde_json::to_value(&f.run).unwrap();
        snapshot["write_access_mode"] = serde_json::to_value(write_access).unwrap();
        let run: crate::provider::RuntimeProviderRun = serde_json::from_value(snapshot).unwrap();
        f.app.lock().await.providers_mut().insert_run_for_test(run);
        f.state
            .owned
            .provider_store
            .update_run_execution_config(f.run.id(), mode, AgentPermissionLevel::Required)
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
        let result = f
            .router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox.events.process",
                serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id,
                "label":"restricted run must not execute", "argv":["/bin/echo","must-not-run"],
                "match_text":"must-not-run"}),
            )
            .await;
        f.state.owned.agent_wakes.processes.shutdown();
        assert!(
            result.is_err(),
            "sudo must retain {mode:?}/{write_access:?}: {result:?}"
        );
    }
}

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
            .sudo_native_permission_resolution(&turn.session_id, &interaction)
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
        .sudo_native_permission_resolution(&turn.session_id, &interaction)
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
async fn sudo_operations_watched_command_reports_linux_ownership_requirement() {
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
    let error = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.process",
            serde_json::json!({"task_id":task.task_id,"origin_prompt_id":turn.prompt_id,
            "label":"unsupported ownership check","argv":["/bin/echo","not-started"]}),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("require Linux process-group ownership"),
        "{error}"
    );
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

const HOOKS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./audit.sh"}]}]}}"#;

fn hooks_launch(
    f: &super::tests::Fixture,
    profile: &std::path::Path,
) -> crate::provider::LaunchProviderRequest {
    let folder = f.run.working_directory().unwrap().to_owned();
    std::fs::create_dir_all(folder.join(".claude")).unwrap();
    std::fs::write(folder.join(".claude/settings.json"), HOOKS).unwrap();
    crate::provider::LaunchProviderRequest::new(
        f.run.session_id(),
        "claude",
        "claude",
        "default",
        "haiku",
    )
    .with_agent_id(f.run.agent_instance_id().unwrap())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
    .with_working_directory(folder)
    .with_permission_level(crate::provider::AgentPermissionLevel::Required)
    .with_provider_account_env(std::collections::BTreeMap::from([(
        "CLAUDE_CONFIG_DIR".to_string(),
        profile.display().to_string(),
    )]))
}

#[tokio::test]
async fn sudo_operations_cold_claude_launch_with_hooks_reuses_the_live_window() {
    let f = fixture();
    let profile = crate::test_support::TestWorktree::new("sudo-hooks-profile");
    let request = hooks_launch(&f, profile.path());
    running(&f);
    tokio::time::timeout(
        Duration::from_secs(2),
        f.state.admit_claude_workspace_trust(&request),
    )
    .await
    .expect("a launch covered by /sudo asked about project hooks")
    .unwrap();
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.path().join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(
        state["projects"][request
            .working_directory
            .as_ref()
            .unwrap()
            .to_str()
            .unwrap()]["hasTrustDialogAccepted"],
        true
    );
}

#[tokio::test]
async fn sudo_operations_cold_claude_launch_in_the_agents_own_worktree_reuses_the_live_window() {
    let f = fixture();
    let profile = crate::test_support::TestWorktree::new("sudo-hooks-profile");
    let agent_tree = crate::test_support::TestWorktree::new("sudo-agent-tree");
    std::fs::create_dir_all(agent_tree.path().join(".claude")).unwrap();
    std::fs::write(agent_tree.path().join(".claude/settings.json"), HOOKS).unwrap();
    let turn = running(&f);
    f.state
        .owned
        .agent_store
        .update_agent_config(
            &turn.agent_id,
            None,
            None,
            None,
            Some(Some(agent_tree.path().display().to_string())),
        )
        .unwrap();
    let request =
        hooks_launch(&f, profile.path()).with_working_directory(agent_tree.path().to_path_buf());
    tokio::time::timeout(
        Duration::from_secs(2),
        f.state.admit_claude_workspace_trust(&request),
    )
    .await
    .expect("a launch in the agent's own worktree covered by /sudo asked about project hooks")
    .unwrap();
}

#[tokio::test]
async fn sudo_operations_forwarded_hooks_card_of_a_cold_launch_reuses_the_live_window() {
    let f = fixture();
    let turn = running(&f);
    let binding = crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".into(),
        worker_machine_id: "worker-machine".into(),
        execution_lease_id: "worker-lease".into(),
        leased_agent_id: "leased-agent".into(),
        active_worker_provider_run_id: None,
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
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&turn.entry_id)
        .unwrap()
        .placement = Some(binding.execution_lease_id.clone());
    let interaction = RuntimeInteraction::new(
        "claude-workspace-hooks-cold",
        &turn.agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Run this folder's Claude hooks?".into()),
        "hooks",
        vec![
            RuntimeInteractionChoice::new("trust", "Trust folder", "trust", None),
            RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
        ],
        None,
        Some(600),
        Some("cancel".into()),
    )
    .with_native_origin(Some(NativeInteractionOrigin::ProviderStartup {
        provider_run_id: "launch-claude-workspace-hooks-cold".into(),
    }));
    let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
        home_session_id: turn.session_id.clone(),
        home_agent_id: turn.agent_id.clone(),
        leased_agent_id: binding.leased_agent_id.clone(),
        worker_provider_run_id: "launch-claude-workspace-hooks-cold".into(),
        home_prompt_id: turn.prompt_id.clone(),
    };
    let answer = tokio::time::timeout(
        Duration::from_secs(2),
        crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
            &f.state,
            context,
            interaction,
        ),
    )
    .await
    .expect("a hooks card covered by /sudo waited for a person")
    .unwrap();
    assert_eq!(answer.choice_id.as_deref(), Some("trust"));
}
