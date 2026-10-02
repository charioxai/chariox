use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};

const PASSKEY: &str = "sudo fixture passkey";

struct Fixture {
    _worktree: crate::test_support::TestWorktree,
    state: KernelRuntimeState,
    router: crate::runtime::router::CommandRouter,
    request: SubmitPromptRequest,
    run: RuntimeProviderRun,
}

fn fixture() -> Fixture {
    let worktree = crate::test_support::TestWorktree::new("sudo-turn");
    let vault = worktree.path().join("test-vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
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
    let launch =
        LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "default")
            .with_agent_id(agent.id());
    let mut run = RuntimeProviderRun::new(
        "sudo-fixture-run",
        &launch,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
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
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some("sudo-fixture-bearer".into()));
    app.providers_mut().insert_run_for_test(run.clone());
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
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app, 32);
    let state = router.runtime_state();
    state.owned.provider_run_projection.update(run.clone());
    Fixture {
        _worktree: worktree,
        state,
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

fn running(f: &Fixture) -> KernelSudoTurn {
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
        prompt_id: Some("sudo-exact-turn".into()),
        provider_run_id: Some(f.run.id().into()),
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

async fn popup(state: &KernelRuntimeState) -> PasskeyPrompt {
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
async fn sudo_exact_turn_ends_at_yield_without_time_expiry_or_inheritance() {
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
async fn sudo_critical_receipt_names_human_entry_and_exact_turn_across_sessions() {
    let f = fixture();
    let turn = running(&f);
    let mut other =
        crate::session::RuntimeSession::new("sudo-other-session", None, "w", "wt", "m", "k");
    // Same host, separate session: sudo has kernel-wide authority.
    other.set_alias(Some("other".into()));
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
                "sudo-critical",
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
    let answer = RespondToInteractionRequest {
        session_id: other.id().into(),
        interaction_id: "sudo-critical".into(),
        choice_id: "approve".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    };
    let grant = f.state.insert_access_grant_for_test(other.id());
    assert!(f
        .state
        .authorize_external_request(
            &grant,
            &LocalDaemonRequest::RespondToInteraction(answer.clone())
        )
        .is_err());
    f.state
        .answer_sudo_interaction(&turn.entry_id, answer.clone())
        .await
        .unwrap();
    assert_eq!(
        responder.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    assert!(f
        .state
        .answer_sudo_interaction(&turn.entry_id, answer)
        .await
        .is_err());
    let receipts = f
        .state
        .owned
        .durable_state_store
        .load_events_by_kind("kernel_access.sudo_approval")
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].payload["turn"]["entry_id"], turn.entry_id);
    assert_eq!(receipts[0].payload["turn"]["prompt_id"], "sudo-exact-turn");
    assert_eq!(receipts[0].payload["turn"]["terminal_id"], "sudo-terminal");
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
        .answer_sudo_interaction(&turn.entry_id, answer)
        .await
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
        prompt_id: None,
        provider_run_id: None,
    };
    f.state.audit_sudo(&entry, "authorized").unwrap();
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
    let resolved = f.router.dispatch_authenticated_runtime_tool_call("sudo-fixture-bearer", "chariox_kernel_request", serde_json::json!({"request":{"ResolveSession":{"session_ref":"sudo-other-alias", "workspace_id":null}}})).await.unwrap();
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
    assert!(f
        .state
        .authorize_sudo_request(
            &turn.entry_id,
            &LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
                path: "workflow.session_default_max_agents".into(),
                value: "16".into()
            })
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
    f.state.revoke_sudo(
        Some("local"),
        Some(&prompt.interaction_id),
        "fixture_cleanup",
    );
}
