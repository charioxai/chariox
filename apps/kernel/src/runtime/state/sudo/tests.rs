use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};

pub(super) const PASSKEY: &str = "sudo fixture passkey";

pub(super) struct Fixture {
    _worktree: crate::test_support::TestWorktree,
    pub(super) state: KernelRuntimeState,
    pub(super) app: Arc<Mutex<crate::app::DaemonApp>>,
    pub(super) router: crate::runtime::router::CommandRouter,
    pub(super) request: SubmitPromptRequest,
    pub(super) run: RuntimeProviderRun,
}

fn fixture() -> Fixture {
    fixture_with_provider(None)
}

pub(super) fn fixture_with_provider(script: Option<&str>) -> Fixture {
    fixture_with_options(script, false)
}

pub(super) fn fixture_with_options(script: Option<&str>, room_tools: bool) -> Fixture {
    fixture_with_run_profile(script, room_tools, "dev-stub", "dev-stub")
}

// MP-08/MP-10/MP-11: metadata-only warm catalog-caching run. Its agent
// remains dev-stub; no real provider credentials or processes are used.
pub(super) fn fixture_with_catalog_reload() -> Fixture {
    fixture_with_run_profile(None, true, "codex", "sudo-relaunch-fixture")
}

pub(super) fn fixture_with_run_profile(
    script: Option<&str>,
    room_tools: bool,
    adapter: &str,
    provider: &str,
) -> Fixture {
    fixture_with_run_endpoint(script, room_tools, adapter, provider, None)
}

pub(super) fn fixture_with_run_endpoint(
    script: Option<&str>,
    room_tools: bool,
    adapter: &str,
    provider: &str,
    endpoint: Option<String>,
) -> Fixture {
    let worktree = crate::test_support::TestWorktree::new("sudo-turn");
    let vault = worktree.path().join("test-vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = room_tools;
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "sudo-terminal",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let launch = LaunchProviderRequest::new(session.id(), adapter, provider, "default", "default")
        .with_agent_id(agent.id());
    let mut run = RuntimeProviderRun::new(
        "sudo-fixture-run",
        &launch,
        ProviderLaunchResult {
            endpoint_mode: if script.is_some() {
                AgentEndpointMode::Managed
            } else {
                AgentEndpointMode::External
            },
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: script.map(|_| "/bin/bash".into()),
            pty_args: script
                .map(|script| vec!["-c".into(), script.into()])
                .unwrap_or_default(),
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.path().to_owned()),
            structured_endpoint: endpoint.clone(),
        },
    );
    if endpoint.is_none() {
        run.mark_running();
    }
    run.set_runtime_mcp_auth_token(Some("sudo-fixture-bearer".into()));
    // The kernel must admit the current run before its process can launch.
    app.providers_mut().insert_run_for_test(run.clone());
    if script.is_some() {
        crate::app::ProviderLaunchProcessRuntime::new(&mut app)
            .spawn_for_launch(&run)
            .unwrap();
    }
    app.agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "dev-stub",
            Some("default".into()),
            Some("default".into()),
            Some("default".into()),
            run.resume_state().clone(),
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    state.owned.provider_run_projection.update(run.clone());
    Fixture {
        _worktree: worktree,
        state,
        app,
        router,
        request: SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: "/sudo protected task".into(),
            attachments: vec![],
        },
        run,
    }
}

// MP-08 / MP-11, P2: exercise the actual MCP router with room tools enabled.
#[tokio::test]
async fn sudo_room_tools_preserve_host_authority_and_revocation() {
    let f = fixture_with_options(None, true);
    assert!(
        f.router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request":{"ListSessions":null}}),
            )
            .await
            .is_err(),
        "room tools alone must not confer sudo authority"
    );
    let turn = running(&f);
    let other =
        crate::session::RuntimeSession::new("sudo-room-tools-other", None, "w", "wt", "m", "k");
    f.state
        .owned
        .session_store
        .write()
        .restore_session(other.clone());
    let responder = f
        .state
        .create_kernel_operation_interaction(
            other.id(),
            "local",
            RuntimeInteraction::for_kernel_operation(
                "sudo-room-tools-critical",
                "payment",
                "Payment",
                "Fixture only",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                        .requiring_passkey(),
                ],
            ),
        )
        .await
        .unwrap();
    let answer = serde_json::json!({"RespondToInteraction": {
        "session_id":other.id(), "interaction_id":"sudo-room-tools-critical",
        "choice_id":"approve", "custom_reply":null, "passkey":null,
        "passkey_remember_minutes":null,
    }});
    assert!(
        f.router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request":answer}),
            )
            .await
            .is_err(),
        "approvals belong to the user, even for an elevated agent"
    );
    drop(responder);
    let peer = {
        let mut app = f.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&turn.session_id, "dev-stub")
                    .with_alias("sudo-room-peer"),
            )
            .unwrap()
    };
    for request in [
        serde_json::json!({"AliasAgent":{"session_id":turn.session_id,
            "agent_id":peer.id(),"alias":"sudo-renamed-peer"}}),
        serde_json::json!({"DestroyAgent":{"session_id":turn.session_id,
            "agent_id":peer.id()}}),
        serde_json::json!({"EndSession":{"session_id":other.id()}}),
    ] {
        assert!(
            f.router
                .dispatch_authenticated_runtime_tool_call(
                    "sudo-fixture-bearer",
                    "chariox_kernel_request",
                    serde_json::json!({"request":request}),
                )
                .await
                .is_err(),
            "sudo cannot take another creator's agents or tear down unrelated sessions"
        );
    }
    // Room tools do not remove the sudo forbidden-operation policy.
    for request in [
        serde_json::json!({"RevokeKernelAccessGrant":{"grant_id":null}}),
        serde_json::json!({"SubmitPrompt":f.request}),
    ] {
        assert!(f
            .router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request":request}),
            )
            .await
            .is_err());
    }
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
    assert!(
        f.router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request":{"ListSessions":null}}),
            )
            .await
            .is_err(),
        "room flag must not revive revoked sudo"
    );
}

pub(super) fn running(f: &Fixture) -> KernelSudoTurn {
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let prompt = PromptQueueItem::new(
        "sudo-exact-turn",
        &f.request.attachment_id,
        agent,
        "protected task",
        PromptStatus::Queued,
    );
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(&session, prompt, false, false)
        .unwrap();
    let turn = KernelSudoTurn {
        entry_id: "sudo:fixture".into(),
        session_id: session.id().into(),
        agent_id: agent.into(),
        owner_user_id: session.owner_user_id().into(),
        terminal_id: "sudo-terminal".into(),
        requester: None,
        prompt_id: Some("sudo-exact-turn".into()),
        provider_run_id: Some(f.run.id().into()),
        task_id: None,
        duration_minutes: 60,
        expires_at_ms: Some(crate::session::unix_epoch_ms() + 3_600_000),
        revision: 1,
        warning_sent: false,
        deadline: Some(std::time::Instant::now() + Duration::from_secs(3600)),
        placement: None,
    };
    assert!(f.state.owned.prompt_state_owner.bind_sudo_turn(
        &session,
        agent,
        "sudo-exact-turn",
        &turn.entry_id
    ));
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .insert(turn.entry_id.clone(), turn.clone());
    turn
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(mut app) = self.app.try_lock() {
            let _ = crate::app::ProviderProcessTracker::new(&mut app).remove_run(self.run.id());
        }
    }
}

pub(super) async fn popup(state: &KernelRuntimeState) -> PasskeyPrompt {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(prompt) = state
                .passkey_prompts_for("local")
                .into_iter()
                .find(|p| p.kind == PasskeyPromptKind::Sudo)
            {
                return prompt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn sudo_without_durable_tasks_ends_with_its_turn() {
    let f = fixture();
    let turn = running(&f);
    assert_eq!(
        f.state.sudo_for_auth_token("sudo-fixture-bearer").unwrap(),
        turn
    );
    assert!(f.state.sudo_for_auth_token("sibling-bearer").is_err());
    assert!(f
        .state
        .authorize_sudo_request(
            &turn.entry_id,
            &LocalDaemonRequest::ListSessions(ListSessionsRequest)
        )
        .is_ok());
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
    let next = PromptQueueItem::new(
        "ordinary-next-turn",
        &f.request.attachment_id,
        &turn.agent_id,
        "ordinary",
        PromptStatus::Queued,
    );
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(&session, next, false, false)
        .unwrap();
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    assert!(f
        .state
        .authorize_sudo_request(
            &turn.entry_id,
            &LocalDaemonRequest::ListSessions(ListSessionsRequest)
        )
        .is_err());
    assert!(f.state.list_sudo_turns("local").is_empty());
}

#[tokio::test]
async fn sudo_cannot_mint_authority_answer_sudo_popup_or_submit_passkeys() {
    let f = fixture();
    let turn = running(&f);
    assert!(f
        .state
        .authorize_sudo_request(
            &turn.entry_id,
            &LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
                grant_id: None
            })
        )
        .is_err());
    assert!(f
        .state
        .authorize_sudo_request(
            &turn.entry_id,
            &LocalDaemonRequest::SubmitPrompt(f.request.clone())
        )
        .is_err());
    let _responder = f
        .state
        .create_kernel_operation_interaction(
            &turn.session_id,
            "local",
            RuntimeInteraction::for_kernel_operation(
                "second-sudo",
                "sudo:second",
                "Sudo",
                "Another request",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                        .requiring_passkey(),
                ],
            ),
        )
        .await
        .unwrap();
    let answer = RespondToInteractionRequest {
        session_id: turn.session_id.clone(),
        interaction_id: "second-sudo".into(),
        choice_id: "approve".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    };
    assert!(f
        .state
        .authorize_sudo_request(
            &turn.entry_id,
            &LocalDaemonRequest::RespondToInteraction(answer)
        )
        .is_err());
}

#[tokio::test]
async fn sudo_cannot_pair_invite_or_read_or_replace_relay_identity() {
    let f = fixture();
    let turn = running(&f);
    for value in [
        serde_json::json!({"CreatePairingInvite":{"intent":"client"}}),
        serde_json::json!({"JoinPairingInvite":{"invite_token":"fixture"}}),
        serde_json::json!({"CreateTerminalPairingLink":{}}),
        serde_json::json!({"JoinTerminalPairingLink":{"pairing_link":"fixture"}}),
        serde_json::json!({"RecordPairedClient":{"client_id":"fixture","public_key_thumbprint":"fixture"}}),
        serde_json::json!({"ApproveRemoteMachine":{"machine_ref":"fixture"}}),
        serde_json::json!({"CreateSessionInvite":{"session_id":"fixture"}}),
        serde_json::json!({"JoinSessionInvite":{"invite_token":"fixture","user_id":"fixture"}}),
        serde_json::json!({"CreateCloudSessionInvite":{"session_id":"fixture"}}),
        serde_json::json!({"AcceptCloudSessionInvite":{"invite_token":"fixture"}}),
        serde_json::json!({"ConfigureRelay":{"relay_url":"wss://fixture.invalid","relay_token":"fixture"}}),
        serde_json::json!({"CloudRelayStatus":null}),
        serde_json::json!({"StartCloudRelayLogin":{"api_url":"https://fixture.invalid"}}),
        serde_json::json!({"PollCloudRelayLogin":{"api_url":"https://fixture.invalid","device_code":"fixture"}}),
        serde_json::json!({"LogoutCloudRelay":{}}),
        serde_json::json!({"PairCloudRelayClient":{"client_id":"fixture"}}),
        serde_json::json!({"PairCloudRelayMachine":{"machine_id":"fixture"}}),
        serde_json::json!({"ConnectCloudRelay":null}),
    ] {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert!(
            f.state
                .authorize_sudo_request(&turn.entry_id, &request)
                .is_err(),
            "sudo admitted {value}"
        );
        // Exercise the actual runtime MCP router as well as the policy check.
        assert!(f
            .router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request": value}),
            )
            .await
            .is_err());
    }
}

#[tokio::test]
async fn sudo_busy_authorization_is_memory_only_and_revoke_all_prevents_dispatch() {
    let f = fixture();
    let running_turn = running(&f);
    f.state.owned.sudo_turns.lock().unwrap().clear(); // Ordinary busy turn.
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    assert!(prompt.message.contains("protected task"));
    assert!(prompt.message.contains(&running_turn.agent_id));
    assert!(f
        .state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            None,
            None,
            Some(KernelConnectionClass::Terminal)
        )
        .await
        .is_err());
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let session = f
        .state
        .owned
        .session_store
        .get_session(&running_turn.session_id)
        .unwrap();
    assert!(f
        .state
        .owned
        .prompt_state_owner
        .state_parts(&session, &running_turn.agent_id)
        .1
        .is_empty());
    f.state
        .revoke_kernel_access(Some("local"), None, "explicit_revoke")
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &running_turn.agent_id)
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &running_turn.agent_id)
        .is_none());
    assert!(f.state.list_sudo_turns("local").is_empty());
}

#[tokio::test]
async fn sudo_guest_and_cancelled_request_leave_no_authorization() {
    let f = fixture();
    assert!(f
        .state
        .submit_sudo_prompt(f.request.clone(), "guest", "guest-terminal")
        .await
        .is_err());
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    popup(&f.state).await;
    task.abort();
    let _ = task.await;
    assert!(f.state.list_sudo_turns("local").is_empty());
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

#[tokio::test]
async fn sudo_interrupt_and_replayed_prompt_cannot_restore_elevation() {
    let f = fixture();
    let turn = running(&f);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let old = f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &turn.agent_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .begin_cancelling_active_prompt(&session, &turn.agent_id)
        .unwrap();
    assert!(!f.state.sudo_live(&turn));
    // A delayed delivery callback cannot re-elevate an interrupted prompt.
    f.state
        .owned
        .prompt_state_owner
        .mark_active_prompt_running(&session, &turn.agent_id)
        .unwrap();
    assert!(!f.state.sudo_live(&turn));
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &turn.agent_id)
        .unwrap();
    // Deliberately restore the exact old id before a sweep. Its ephemeral
    // binding was consumed at yield, so even this stronger replay is refused.
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(&session, old, false, false)
        .unwrap();
    assert!(!f.state.sudo_live(&turn));
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
}

#[tokio::test]
async fn sudo_revoke_interrupts_exact_turn_and_guest_cannot_revoke_it() {
    let f = fixture();
    let turn = running(&f);
    assert_eq!(
        f.state
            .revoke_kernel_access(Some("guest"), None, "explicit_revoke")
            .unwrap(),
        0
    );
    assert!(f.state.sudo_live(&turn));
    assert_eq!(
        f.state
            .revoke_kernel_access(Some("local"), Some(&turn.entry_id), "explicit_revoke")
            .unwrap(),
        1
    );
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let active = f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &turn.agent_id)
        .unwrap();
    assert_eq!(active.id(), turn.prompt_id.as_deref().unwrap());
    assert_eq!(active.status(), PromptStatus::Cancelling);
}

#[tokio::test]
async fn sudo_rotation_cancels_authorized_queue_before_idle_dispatch() {
    let f = fixture();
    let busy = running(&f);
    f.state.owned.sudo_turns.lock().unwrap().clear();
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.state
        .change_vault_passphrase(
            "local",
            &f._worktree.path().join("test-vault.json"),
            zeroize::Zeroizing::new(PASSKEY.into()),
            zeroize::Zeroizing::new("rotated sudo fixture".into()),
        )
        .await
        .unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&busy.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &busy.agent_id)
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &busy.agent_id)
        .is_none());
}

#[tokio::test]
async fn sudo_restart_discards_queue_and_records_notice_without_prompt_content() {
    let f = fixture();
    let entry = KernelSudoTurn {
        entry_id: "sudo:restart-pending".into(),
        session_id: f.request.session_id.clone(),
        agent_id: f.request.target_agent_id.clone().unwrap(),
        owner_user_id: "local".into(),
        terminal_id: "sudo-terminal".into(),
        requester: None,
        prompt_id: None,
        provider_run_id: None,
        task_id: None,
        duration_minutes: 60,
        expires_at_ms: None,
        revision: 1,
        warning_sent: false,
        deadline: None,
        placement: None,
    };
    f.state.audit_sudo(&entry, "extended").unwrap();
    // Recover only the audit stream, as a new kernel does. It never creates
    // a live authorization or persists the queued prompt's text.
    f.state.recover_sudo_notices();
    assert!(f.state.list_sudo_turns("local").is_empty());
    let events = f
        .state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&entry.entry_id, "kernel_access.sudo", 10)
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.payload["outcome"] == "restart_dropped"));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("protected task"));
}

#[tokio::test]
async fn sudo_runtime_mcp_uses_shared_router_and_removes_tool_at_yield() {
    let f = fixture();
    // MP-08/MP-10/MP-11: ordinary turns do not advertise the sudo tool.
    // The authorized turn refreshes cached catalogs before dispatch, and
    // both sides of the turn still recheck live authority on every call.
    let initial_catalog = f
        .router
        .runtime_tool_specs_for_auth_token("sudo-fixture-bearer");
    assert!(!f
        .router
        .runtime_tool_specs_for_auth_token("unknown-token")
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    assert!(!initial_catalog
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    assert!(f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request": {"ListSessions": null}})
        )
        .await
        .is_err());
    let turn = running(&f);
    assert!(f
        .router
        .runtime_tool_specs_for_auth_token("sudo-fixture-bearer")
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    let result = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request": {"ListSessions": null}}),
        )
        .await
        .unwrap();
    assert!(result.ok);
    assert!(result.payload.get("SessionsListed").is_some());
    let mut other =
        crate::session::RuntimeSession::new("sudo-router-other", None, "w", "wt", "m", "k");
    other.set_alias(Some("sudo-other-alias".into()));
    f.state
        .owned
        .session_store
        .write()
        .restore_session(other.clone());
    let router = f.router.clone();
    let resolving = tokio::spawn(async move {
        router.dispatch_authenticated_runtime_tool_call("sudo-fixture-bearer", "chariox_kernel_request", serde_json::json!({"request":{"ResolveSession":{"session_ref":"sudo-other-alias", "workspace_id":null}}})).await
    });
    let scope_prompt = popup(&f.state).await;
    assert!(scope_prompt.interaction_id.contains(":scope:"));
    f.state
        .answer_terminal_runtime_interaction(
            &turn.session_id,
            &scope_prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    let resolved = tokio::time::timeout(Duration::from_secs(5), resolving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.payload["SessionResolved"]["session"]["id"],
        other.id()
    );

    for path in [
        "kernel_access.request_timeout_minutes",
        "credential_vault.path",
        "relay.url",
        "relay.accept_remote_leases",
    ] {
        assert!(f
            .state
            .authorize_sudo_request(
                &turn.entry_id,
                &LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
                    path: path.into(),
                    value: "invalid".into()
                })
            )
            .is_err());
    }
    assert!(
        f.state
            .authorize_sudo_request(
                &turn.entry_id,
                &LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
                    path: "workflow.session_default_max_agents".into(),
                    value: "16".into()
                })
            )
            .is_err(),
        "an unclassified host mutation needs explicit owner scope authorization"
    );
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
    assert!(!f
        .router
        .runtime_tool_specs_for_auth_token("sudo-fixture-bearer")
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    assert!(f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request": {"ListSessions": null}})
        )
        .await
        .is_err());
}

#[tokio::test]
async fn sudo_fresh_popup_starts_one_separate_turn_through_normal_admission() {
    let f = fixture();
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let LocalDaemonResponse::PromptSubmitted {
        outcome: PromptSubmissionOutcome::Started { prompt: started },
        ..
    } = response
    else {
        panic!("sudo should start separately");
    };
    assert_ne!(started.id(), prompt.interaction_id);
    assert_eq!(started.prompt(), "protected task");
    let events = f
        .state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&prompt.interaction_id, "kernel_access.sudo", 10)
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.payload["outcome"] == "started"
            && event.payload["turn"]["prompt_id"] == started.id()));
    // The metadata-only run cannot execute effects. End any admitted fixture turn.
    f.state
        .revoke_sudo(
            Some("local"),
            Some(&prompt.interaction_id),
            "fixture_cleanup",
        )
        .unwrap();
}

fn external_request(f: &Fixture) -> RequestKernelSudoRequest {
    RequestKernelSudoRequest {
        agent_id: f.request.target_agent_id.clone().unwrap(),
        prompt: "  external first line\nfull second line\t  \n".into(),
    }
}

#[test]
fn sudo_command_parser_shares_router_whitespace_and_token_boundaries() {
    for (text, expected) in [
        ("  /sudo task  ", Some("task")),
        ("/sudo\ntask", Some("task")),
        ("/sudo\ttask", Some("task")),
        ("\n/sudo\u{2003}task", Some("task")),
        ("/sudo", Some("")),
        ("/sudo \t\n", Some("")),
        ("/sudo-task", None),
        ("/sudotask", None),
        ("please /sudo task", None),
    ] {
        assert_eq!(policy::parse_sudo_prompt(text), expected, "{text:?}");
        assert_eq!(is_sudo_prompt(text), expected.is_some(), "{text:?}");
    }
}

async fn external_sudo_submit_prompt_whitespace_is_refusable(text: &str) {
    let f = fixture();
    let grant_id = f.state.insert_access_grant_for_test(&f.request.session_id);
    let mut request = f.request.clone();
    request.prompt = text.into();
    let request = LocalDaemonRequest::SubmitPrompt(request);
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "external-sudo-whitespace",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant_id;
    let router = f.router.clone();
    let task = tokio::spawn(async move { router.dispatch(command, request).await });
    let prompt = popup(&f.state).await;
    assert!(prompt
        .message
        .ends_with("Requester-supplied prompt:\nprotected task\nfull second line"));
    assert!(prompt.message.contains("External agent"));
    f.state
        .answer_sudo_entry_from_terminal(
            "local",
            "sudo-terminal",
            &RespondToInteractionRequest {
                session_id: prompt.session_id.clone(),
                interaction_id: prompt.interaction_id.clone(),
                choice_id: "refuse".into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            },
        )
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("sudo request refused"));
    assert!(f.state.passkey_prompts_for("local").is_empty());
    assert!(f.state.list_sudo_turns("local").is_empty());
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let (active, queued) = f
        .state
        .owned
        .prompt_state_owner
        .state_parts(&session, f.request.target_agent_id.as_deref().unwrap());
    assert!(active.is_none());
    assert!(queued.is_empty());
}

#[tokio::test]
async fn external_sudo_submit_prompt_accepts_leading_whitespace() {
    external_sudo_submit_prompt_whitespace_is_refusable(
        "  /sudo protected task\nfull second line  ",
    )
    .await;
}

#[tokio::test]
async fn external_sudo_submit_prompt_accepts_newline_separator() {
    external_sudo_submit_prompt_whitespace_is_refusable("/sudo\nprotected task\nfull second line")
        .await;
}

#[tokio::test]
async fn external_sudo_submit_prompt_accepts_tab_separator() {
    external_sudo_submit_prompt_whitespace_is_refusable("/sudo\tprotected task\nfull second line")
        .await;
}

#[tokio::test]
async fn external_sudo_popup_names_os_requester_target_session_and_full_prompt() {
    let f = fixture();
    let grant_id = f.state.insert_access_grant_for_test(&f.request.session_id);
    let request = external_request(&f);
    let state = f.state.clone();
    let id = grant_id.clone();
    let task = tokio::spawn(async move { state.request_kernel_sudo(&id, request).await });
    let prompt = popup(&f.state).await;
    assert!(prompt.message.contains("External agent"));
    assert!(prompt
        .message
        .contains(&format!("OS pid {}", std::process::id())));
    assert!(prompt.message.contains(
        &crate::runtime::kernel_access::process::inspect(std::process::id())
            .unwrap()
            .0
            .executable
    ));
    assert!(prompt
        .message
        .contains(f.request.target_agent_id.as_deref().unwrap()));
    assert!(prompt.message.contains(&f.request.session_id));
    assert!(prompt
        .message
        .ends_with("Requester-supplied prompt:\n  external first line\nfull second line\t  \n"));
    assert!(f.state.passkey_prompts_for("guest").is_empty());
    let mut answer = RespondToInteractionRequest {
        session_id: prompt.session_id.clone(),
        interaction_id: prompt.interaction_id.clone(),
        choice_id: "approve".into(),
        custom_reply: None,
        passkey: Some(ApprovalPasskey::new(PASSKEY)),
        passkey_remember_minutes: None,
    };
    assert!(f
        .state
        .authorize_external_request(
            &grant_id,
            &LocalDaemonRequest::RespondToInteraction(answer.clone())
        )
        .is_err());
    assert!(f
        .state
        .answer_sudo_entry_from_terminal("guest", "guest-terminal", &answer)
        .await
        .is_err());
    assert!(f
        .state
        .request_kernel_sudo(&grant_id, external_request(&f))
        .await
        .is_err());
    f.state
        .answer_sudo_entry_from_terminal("local", "approving-host-terminal", &answer)
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        LocalDaemonResponse::KernelSudoRequested { .. }
    ));
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let (active, _) = f
        .state
        .owned
        .prompt_state_owner
        .state_parts(&session, f.request.target_agent_id.as_deref().unwrap());
    assert_eq!(
        active.unwrap().prompt(),
        "  external first line\nfull second line\t  \n"
    );
    let events = f
        .state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&prompt.interaction_id, "kernel_access.sudo", 10)
        .unwrap();
    let started = events
        .iter()
        .find(|event| event.payload["outcome"] == "started")
        .unwrap();
    assert_eq!(
        started.payload["turn"]["terminal_id"],
        "approving-host-terminal"
    );
    assert_eq!(started.payload["turn"]["requester"]["grant_id"], grant_id);
    assert!(!serde_json::to_string(&events).unwrap().contains(PASSKEY));
    answer.passkey = None;
    assert!(f
        .state
        .answer_sudo_entry_from_terminal("local", "other-terminal", &answer)
        .await
        .is_err());
    f.state
        .revoke_sudo(Some("local"), None, "fixture_cleanup")
        .unwrap();
    assert_eq!(
        f.state
            .owned
            .attachment_store
            .list_session_attachment_ids(&f.request.session_id)
            .len(),
        1
    );
}

#[tokio::test]
async fn external_sudo_revoked_holder_cancels_popup_and_busy_queue() {
    for authorized in [false, true] {
        let f = fixture();
        let busy = running(&f);
        f.state.owned.sudo_turns.lock().unwrap().clear();
        let grant_id = f.state.insert_access_grant_for_test(&f.request.session_id);
        let request = external_request(&f);
        let state = f.state.clone();
        let id = grant_id.clone();
        let task = tokio::spawn(async move { state.request_kernel_sudo(&id, request).await });
        let prompt = popup(&f.state).await;
        if authorized {
            f.state
                .answer_sudo_entry_from_terminal(
                    "local",
                    "host",
                    &RespondToInteractionRequest {
                        session_id: prompt.session_id.clone(),
                        interaction_id: prompt.interaction_id.clone(),
                        choice_id: "approve".into(),
                        custom_reply: None,
                        passkey: Some(ApprovalPasskey::new(PASSKEY)),
                        passkey_remember_minutes: None,
                    },
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        f.state
            .revoke_kernel_access(Some("local"), Some(&grant_id), "explicit_revoke")
            .unwrap();
        f.state.sweep_kernel_access();
        let session = f
            .state
            .owned
            .session_store
            .get_session(&busy.session_id)
            .unwrap();
        f.state
            .owned
            .prompt_state_owner
            .cancel_active_prompt_only(&session, &busy.agent_id)
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(f.state.list_sudo_turns("local").is_empty());
        assert!(f.state.passkey_prompts_for("local").is_empty());
        assert_eq!(
            f.state
                .owned
                .attachment_store
                .list_session_attachment_ids(&f.request.session_id)
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn external_sudo_cross_session_request_is_allowed_and_terminal_cannot_request() {
    let f = fixture();
    let request = external_request(&f);
    assert!(f
        .state
        .request_kernel_sudo("missing", request.clone())
        .await
        .is_err());
    let other = crate::session::RuntimeSession::new("ungranted", None, "w", "wt", "m", "k");
    f.state.owned.session_store.write().restore_session(other);
    let grant = f.state.insert_access_grant_for_test("ungranted");
    assert!(f
        .state
        .authorize_external_request(
            &grant,
            &LocalDaemonRequest::RequestKernelSudo(request.clone())
        )
        .is_ok());
    let request = LocalDaemonRequest::RequestKernelSudo(request);
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "external-sudo-tcp",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(KernelConnectionClass::Terminal);
    assert!(f
        .router
        .dispatch(command, request)
        .await
        .unwrap_err()
        .to_string()
        .contains("ws+unix://"));
}

#[tokio::test]
async fn sudo_rotation_interrupts_running_turn_and_revokes_grant() {
    let f = fixture();
    let turn = running(&f);
    f.state.insert_access_grant_for_test(&f.request.session_id);
    f.state
        .change_vault_passphrase(
            "local",
            &f._worktree.path().join("test-vault.json"),
            zeroize::Zeroizing::new(PASSKEY.into()),
            zeroize::Zeroizing::new("rotated fixture".into()),
        )
        .await
        .unwrap();
    assert!(f.state.list_kernel_access("local").is_empty());
    assert!(f.state.list_sudo_turns("local").is_empty());
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    assert_eq!(
        f.state
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &turn.agent_id)
            .unwrap()
            .status(),
        PromptStatus::Cancelling
    );
    let events = f
        .state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&turn.entry_id, "kernel_access.sudo", 10)
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.payload["outcome"] == "passkey_rotation"));
}

#[tokio::test]
async fn sudo_session_end_removes_running_and_pending_authorizations() {
    for pending in [false, true] {
        let f = fixture();
        let task = if pending {
            let state = f.state.clone();
            let request = f.request.clone();
            let task =
                tokio::spawn(
                    async move { state.submit_sudo_prompt(request, "local", "host").await },
                );
            popup(&f.state).await;
            Some(task)
        } else {
            running(&f);
            None
        };
        f.state.end_session(&f.request.session_id).await.unwrap();
        assert!(f.state.list_sudo_turns("local").is_empty());
        assert!(f.state.passkey_prompts_for("local").is_empty());
        if let Some(task) = task {
            assert!(tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .is_err());
        }
    }
}

#[tokio::test]
async fn external_sudo_source_cannot_reopen_a_session_ended_before_attach() {
    let f = fixture();
    f.state.end_session(&f.request.session_id).await.unwrap();
    // Model end_session winning between grant admission and source attachment.
    assert!(f
        .state
        .attach_external_sudo_source(&f.request.session_id, "local")
        .is_err());
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    assert_eq!(session.status(), SessionStatus::Ended);
    assert!(f
        .state
        .owned
        .attachment_store
        .list_session_attachment_ids(&f.request.session_id)
        .is_empty());
}

#[tokio::test]
async fn sudo_admission_waiting_for_grants_does_not_block_an_owner_decision() {
    let f = fixture();
    let running_turn = running(&f);
    let responder = f
        .state
        .create_kernel_operation_interaction(
            &f.request.session_id,
            "local",
            RuntimeInteraction::for_kernel_operation(
                "sudo-lock-order-approval",
                "payment",
                "Payment",
                "Fixture only",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                        .requiring_passkey(),
                ],
            ),
        )
        .await
        .unwrap();
    let grant_id = f.state.insert_access_grant_for_test(&f.request.session_id);
    let mut pending = running_turn.clone();
    pending.entry_id = "sudo:queued-lock-order".into();
    pending.prompt_id = None;
    pending.provider_run_id = None;
    pending.requester = Some(
        f.state.owned.kernel_access.lock().unwrap().grants[&grant_id]
            .summary
            .clone(),
    );
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .insert(pending.entry_id.clone(), pending.clone());
    let mut admitted = pending.clone();
    admitted.prompt_id = Some("next-fixture-prompt".into());
    admitted.provider_run_id = Some(f.run.id().into());
    // A busy grant writer stalls final admission, as the transport sweep can.
    let grants = f.state.owned.kernel_access.lock().unwrap();
    let (entered, enter) = std::sync::mpsc::channel();
    let state = f.state.clone();
    let admission = std::thread::spawn(move || {
        entered.send(()).unwrap();
        state.admit_sudo_turn(&pending, &admitted, true)
    });
    enter.recv_timeout(Duration::from_secs(5)).unwrap();
    // Let the admission thread reach its blocked grant writer.
    std::thread::sleep(Duration::from_millis(100));
    let (completed, completion) = std::sync::mpsc::channel();
    let state = f.state.clone();
    let turn = running_turn.clone();
    let approval = std::thread::spawn(move || {
        let result = state.owned.resolve_runtime_interaction_authorized(
            &turn.session_id,
            "sudo-lock-order-approval",
            "approve",
            None,
            Some("local"),
            true,
            None,
            false,
        );
        completed.send(result).unwrap();
    });
    let answered_while_grants_busy = completion.recv_timeout(Duration::from_secs(2));
    // Release before asserting: the opposite lock order must fail this test
    // without leaving its threads blocked in the rest of the test process.
    drop(grants);
    admission.join().unwrap().unwrap();
    approval.join().unwrap();
    answered_while_grants_busy
        .expect("grant contention blocked an owner decision")
        .unwrap();
    assert_eq!(
        responder.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
}

#[tokio::test]
async fn sudo_bound_turn_refuses_queued_steering_from_terminals_and_external_grants() {
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
        .submit_prepared_prompt_with_queue_policy(
            &session,
            PromptQueueItem::new(
                "untrusted-queued",
                &f.request.attachment_id,
                &turn.agent_id,
                "injected privileged instructions",
                PromptStatus::Queued,
            ),
            true,
            true,
        )
        .unwrap();
    let queued_id = f
        .state
        .owned
        .prompt_state_owner
        .state_parts(&session, &turn.agent_id)
        .1[0]
        .id()
        .to_owned();
    let grant = f.state.insert_access_grant_for_test(&turn.session_id);
    let request = LocalDaemonRequest::SteerQueuedPrompt(crate::local::SteerQueuedPromptRequest {
        session_id: turn.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: turn.agent_id.clone(),
        prompt_id: queued_id.clone(),
    });
    for authority in [None, Some((grant.as_str(), &request))] {
        let error = f
            .state
            .steer_queued_prompt_with_external_authority(
                &turn.session_id,
                &turn.agent_id,
                &f.request.attachment_id,
                &queued_id,
                authority,
            )
            .await
            .err()
            .expect("sudo steering must be refused");
        assert!(error.to_string().contains("sudo"), "{error}");
        let (active, queued) = f
            .state
            .owned
            .prompt_state_owner
            .state_parts(&session, &turn.agent_id);
        assert_eq!(active.unwrap().id(), "sudo-exact-turn");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].id(), queued_id);
        assert_eq!(
            f.state.sudo_for_auth_token("sudo-fixture-bearer").unwrap(),
            turn
        );
    }
}

#[tokio::test]
async fn sudo_bound_turn_refuses_messages_from_another_running_agent() {
    let f = fixture();
    let turn = running(&f);
    let sender = f
        .state
        .owned
        .agent_store
        .create_agent(
            crate::agent::CreateAgentRequest::new(&turn.session_id, "dev-stub"),
            &mut f.state.owned.session_store.write(),
        )
        .unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(
            &session,
            PromptQueueItem::new(
                "ordinary-sender",
                &f.request.attachment_id,
                sender.id(),
                "ordinary task",
                PromptStatus::Queued,
            ),
            false,
            false,
        )
        .unwrap();
    let launch =
        LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "default")
            .with_agent_id(sender.id());
    let mut sender_run = RuntimeProviderRun::new(
        "ordinary-sender-run",
        &launch,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(f._worktree.path().to_owned()),
            structured_endpoint: None,
        },
    );
    sender_run.mark_running();
    sender_run.set_runtime_mcp_auth_token(Some("ordinary-sender-bearer".into()));
    f.state
        .owned
        .provider_store
        .write()
        .insert_run_for_test(sender_run.clone());
    f.state.owned.provider_run_projection.update(sender_run);
    let result = f.state.dispatch_authenticated_runtime_tool_call(
        "ordinary-sender-bearer", crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
        serde_json::json!({"agent": turn.agent_id, "message": "injected privileged instructions",
            "origin_prompt_id": "ordinary-sender"}),
    ).await;
    assert!(matches!(result, Err(error) if error.to_string().contains("sudo")));
    let (active, queued) = f
        .state
        .owned
        .prompt_state_owner
        .state_parts(&session, &turn.agent_id);
    assert_eq!(active.unwrap().prompt(), "protected task");
    assert!(queued.is_empty());
    assert_eq!(
        f.state.sudo_for_auth_token("sudo-fixture-bearer").unwrap(),
        turn
    );
}

#[tokio::test]
async fn sudo_revocation_removes_authority_and_reports_failed_durable_end_receipt() {
    let f = fixture();
    let turn = running(&f);
    f.state.audit_sudo(&turn, "started").unwrap();
    let database = rusqlite::Connection::open(f.state.owned.durable_state_store.path()).unwrap();
    database
        .execute_batch(
            "CREATE TRIGGER reject_sudo_end
        BEFORE INSERT ON durable_state_events
        WHEN NEW.kind = 'kernel_access.sudo'
        BEGIN SELECT RAISE(ABORT, 'injected sudo receipt failure'); END;",
        )
        .unwrap();
    let result = f
        .state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke");
    assert!(
        result.is_err(),
        "the caller must see a failed durable receipt"
    );
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    database
        .execute_batch("DROP TRIGGER reject_sudo_end;")
        .unwrap();
    f.state.recover_sudo_notices();
    let events = f
        .state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&turn.entry_id, "kernel_access.sudo", 10)
        .unwrap();
    assert_eq!(events.last().unwrap().payload["outcome"], "restart_dropped");
}

// MP-08/MP-10/MP-11 F4: executable labels stay on one kernel-authored line.
#[tokio::test]
async fn sudo_review_requester_path_has_no_control_characters() {
    let f = fixture();
    let grant_id = f.state.insert_access_grant_for_test(&f.request.session_id);
    let path = "/workspace/provider\nlabel\tend";
    f.state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .get_mut(&grant_id)
        .unwrap()
        .summary
        .holder_executable = path.into();
    let state = f.state.clone();
    let request = external_request(&f);
    let task = tokio::spawn(async move { state.request_kernel_sudo(&grant_id, request).await });
    let prompt = popup(&f.state).await;
    let prelude = prompt
        .message
        .split("Requester-supplied prompt:\n")
        .next()
        .unwrap();
    let clean =
        !prelude.contains(['\n', '\r', '\t']) && prelude.contains(&path.escape_debug().to_string());
    f.state
        .revoke_sudo(Some("local"), None, "fixture_cleanup")
        .unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(clean, "requester executable must be escaped in the prelude");
}

// MP-08/MP-10/MP-11 F6: auth and enrollment never reach scope approval.
#[test]
fn sudo_review_auth_and_credential_enrollment_are_forbidden() {
    for value in [
        serde_json::json!({"StartProviderLogin":{"provider":"codex","account_profile":"default"}}),
        serde_json::json!({"SendProviderLoginInput":{"login_id":"fixture","data_base64":""}}),
        serde_json::json!({"StartSliceProviderLogin":{"slice_ref":"fixture","provider":"codex","account_profile":"default"}}),
        serde_json::json!({"ImportSliceProviderAuth":{"slice_ref":"fixture","provider":"codex","account_profile":"default"}}),
        serde_json::json!({"RequestCredentialEnrollmentInteraction":{"session_id":"fixture","agent_id":"fixture","enrollment_id":"fixture","profile_id":"fixture","target_version":1,"provider_authorization_url":"https://example.com"}}),
        serde_json::json!({"ArmDeploymentCredentialEnrollment":{"session_id":"fixture","attachment_id":"fixture","agent_id":"fixture","enrollment_id":"fixture","profile_id":"fixture","target_version":1}}),
        serde_json::json!({"PrepareManagedEnvironmentGitCredentialEnrollment":{"environmentId":"fixture","sourceTargetId":"fixture","gitCredentials":{"kind":"none"}}}),
    ] {
        let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
        assert!(
            super::policy::sudo_request_forbidden(&request),
            "credential operation must be forbidden"
        );
    }
}

// MP-08/MP-10/MP-11 F7: controls cannot open an authorization window.
#[tokio::test]
async fn sudo_review_controls_are_not_elevation_prompts() {
    for text in [
        "/sudo status",
        "/sudo extend",
        "/sudo revoke",
        "/sudo status sudo:00af",
        " /sudo\trevoke sudo:00af",
    ] {
        let f = fixture();
        let mut request = f.request.clone();
        request.prompt = text.into();
        assert!(
            !is_sudo_prompt(text),
            "control classified as prompt: {text}"
        );
        assert!(f
            .state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
            .is_err());
        assert!(f.state.passkey_prompts_for("local").is_empty());
    }
    assert!(is_sudo_prompt("/sudo deploy the app"));
    assert!(!is_sudo_prompt("/sudoers"));
}

// MP-08/MP-10/MP-11 P2: control verbs inside task text still require approval.
#[tokio::test]
async fn sudo_review_control_verbs_in_tasks_reach_authorization() {
    for text in [
        "/sudo extend the database schema",
        "/sudo status of prod please",
        "/sudo revoke the obsolete deployment",
        "/sudo status all",
        "/sudo extend sudo:00af extra task text",
        "/sudo status sudo:NOT_HEX",
    ] {
        assert!(is_sudo_prompt(text), "task classified as control: {text}");
        let f = fixture();
        let mut request = f.request.clone();
        request.prompt = text.into();
        let state = f.state.clone();
        let task = tokio::spawn(async move {
            state
                .submit_sudo_prompt(request, "local", "sudo-terminal")
                .await
        });
        let approval = popup(&f.state).await;
        assert!(approval.message.contains(text.trim_start_matches("/sudo ")));
        f.state.end_session(&f.request.session_id).await.unwrap();
        assert!(task.await.unwrap().is_err());
    }
}
