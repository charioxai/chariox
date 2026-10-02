use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::command::KernelCommand;
use crate::runtime::session_actor::FocusedAgentProjection;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn kernel_access_revocation_during_meta_credential_wait_keeps_run_and_task_unchanged() {
    const PASSKEY: &str = "Meta Authority Test Passkey";
    let root = crate::test_support::TestWorktree::new("access-meta-vault");
    let worktree = crate::test_support::TestWorktree::new("access-meta-authority");
    let vault = root.path().join("vault.json");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.to_string_lossy().into_owned();
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::KernelInit;
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY).unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &vault,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config.clone()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut daemon)
        .attach(AttachRequest::new(
            session.id(),
            "meta-terminal",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let profile = daemon
        .provider_account_profile_registry()
        .get(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "default")
        .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &profile.profile_id,
        "synthetic-meta-test-token",
        false,
    )
    .unwrap();
    // Metadata reaches the real reload/credential path without starting a CLI.
    let request = LaunchProviderRequest::new(session.id(), "claude", "claude", "default", "sonnet")
        .with_agent_id(agent.id())
        .with_working_directory(worktree.path().to_owned());
    let mut original = RuntimeProviderRun::new(
        "meta-original-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.path().to_owned()),
            structured_endpoint: None,
        },
    );
    original.mark_running();
    daemon.providers_mut().insert_run_for_test(original.clone());
    daemon.update_provider_run_projection(original.clone());
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
    let app = Arc::new(Mutex::new(daemon));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    let runtime = AgentRuntime::new(
        state.clone(),
        state.provider_runtime_lanes.clone(),
        FocusedAgentProjection::default(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
        state.owned.prompt_state_owner.clone(),
        Default::default(),
    );
    let grant = state.insert_access_grant_for_test(session.id());
    let request = crate::local::SubmitPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        target_agent_id: Some(agent.id().into()),
        prompt: "/meta verify this task".into(),
        attachments: vec![],
    };
    let local = LocalDaemonRequest::SubmitPrompt(request.clone());
    let mut command = KernelCommand::from_local_request("external-meta-wait", None, None, &local);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        let request = request.clone();
        async move { runtime.dispatch_prompt_submit(&command, request).await }
    });
    let interaction = timeout(Duration::from_secs(5), async {
        loop {
            let found = state
                .owned
                .pending_interactions
                .write()
                .keys()
                .find(|id| id.starts_with("vault-unlock-"))
                .cloned();
            if let Some(id) = found {
                break id;
            }
            assert!(
                !pending.is_finished(),
                "Meta submission finished before its credential wait"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    state
        .answer_terminal_runtime_interaction(
            session.id(),
            &interaction,
            "passphrase",
            Some(PASSKEY),
            Some(crate::session::DEFAULT_LOCAL_USER_ID),
            None,
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    let error = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("grant revoked or expired"),
        "{error}"
    );
    let after = state.session_snapshot(session.id()).await.unwrap();
    assert!(
        after.metaagent_task(agent.id()).is_none(),
        "revoked Meta task was created"
    );
    assert!(
        !state
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .is_metaagent(),
        "revoked Meta mode remained active"
    );
    assert_eq!(
        state
            .owned
            .provider_store
            .get_run(original.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running,
        "revoked Meta activation terminated its provider"
    );
    assert!(
        !state
            .owned
            .pending_provider_reloads
            .write()
            .contains_key(agent.id()),
        "revoked Meta activation left a deferred reload"
    );

    // A real dev-stub run gives the terminal control an executable prompt path.
    let terminal_run = {
        let mut app = app.lock().await;
        let run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .unwrap();
        app.update_provider_run_projection(run.clone());
        run
    };
    let terminal = KernelCommand::from_local_request("terminal-meta-control", None, None, &local);
    let response = runtime
        .dispatch_prompt_submit(&terminal, request)
        .await
        .unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::PromptSubmitted { .. }
    ));
    assert!(state
        .session_snapshot(session.id())
        .await
        .unwrap()
        .metaagent_task(agent.id())
        .is_some());
    assert_eq!(
        state
            .owned
            .provider_store
            .get_run(terminal_run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
}

#[tokio::test]
async fn kernel_access_revocation_drops_deferred_prompt_policy_reload() {
    let (worktree, app, state, session, agent, attachment) = policy_fixture("access-meta-deferred");
    let request =
        LaunchProviderRequest::new(session.id(), "claude", "dev-stub", "default", "model")
            .with_agent_id(agent.id());
    let mut original = RuntimeProviderRun::new(
        "deferred-original",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.path().to_owned()),
            structured_endpoint: None,
        },
    );
    original.mark_running();
    state
        .owned
        .provider_store
        .write()
        .insert_run_for_test(original.clone());
    state.owned.provider_run_projection.update(original.clone());
    app.lock()
        .await
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "busy-policy-prompt",
                attachment.id(),
                agent.id(),
                "busy",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
    let grant = state.insert_access_grant_for_test(session.id());
    let local = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        target_agent_id: Some(agent.id().into()),
        prompt: "/meta task".into(),
        attachments: vec![],
    });
    let scoped = state.with_external_command_authority(Some((&grant, &local)));
    assert_eq!(
        scoped
            .reload_agent_provider_for_policy(session.id(), agent.id(), "meta mode activation")
            .await
            .unwrap(),
        super::super::super::ProviderReloadOutcome::Deferred
    );
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    timeout(Duration::from_secs(5), async {
        while state
            .owned
            .pending_provider_reloads
            .write()
            .contains_key(agent.id())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        state
            .session_snapshot(session.id())
            .await
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .is_some(),
        "busy prompt must remain active during invalidation"
    );
    assert_eq!(
        state
            .owned
            .provider_store
            .get_run(original.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running,
        "revoked deferred reload terminated a provider"
    );
}

#[tokio::test]
async fn kernel_access_revocation_blocks_prompt_policy_relaunch_at_app_wait() {
    let (_worktree, app, mut state, session, agent, attachment) =
        policy_fixture("access-meta-relaunch");
    let grant = state.insert_access_grant_for_test(session.id());
    let local = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        target_agent_id: Some(agent.id().into()),
        prompt: "/meta task".into(),
        attachments: vec![],
    });
    let probe = Arc::new(tokio::sync::Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let scoped = state.with_external_command_authority(Some((&grant, &local)));
    let held = app.lock().await;
    scoped.spawn_provider_relaunch(
        LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "model")
            .with_agent_id(agent.id()),
        0,
        None,
        0,
    );
    timeout(Duration::from_secs(5), probe.notified())
        .await
        .unwrap();
    let runs = state.owned.provider_store.list_runs();
    assert_eq!(runs.len(), 1);
    let run_id = runs[0].id().to_owned();
    assert_eq!(runs[0].state(), crate::provider::ProviderRunState::Starting);
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(held);
    timeout(Duration::from_secs(5), async {
        loop {
            if state.owned.provider_store.get_run(&run_id).unwrap().state()
                == crate::provider::ProviderRunState::Ended
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(state
        .owned
        .provider_store
        .list_runs()
        .iter()
        .all(|run| run.state() == crate::provider::ProviderRunState::Ended));
}

fn policy_fixture(
    label: &str,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<crate::DaemonApp>>,
    KernelRuntimeState,
    crate::session::RuntimeSession,
    crate::agent::AgentInstance,
    crate::attachment::RuntimeAttachment,
) {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut daemon =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut daemon)
        .attach(AttachRequest::new(
            session.id(),
            "policy-holder",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let app = Arc::new(Mutex::new(daemon));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    (
        worktree,
        app,
        router.runtime_state(),
        session,
        agent,
        attachment,
    )
}
