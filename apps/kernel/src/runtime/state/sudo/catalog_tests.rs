//! MP-08/MP-10/MP-11: an idle native run may still have a busy catalog lane.
use super::relaunch_tests::queued_reload;
use super::tests::{
    fixture_with_catalog_reload, fixture_with_options, fixture_with_run_profile, popup, running,
    Fixture, PASSKEY,
};
use super::*;
use crate::provider::{
    AgentEndpointMode, ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
};

async fn native_fixture(room_tools: bool) -> Fixture {
    native_fixture_for(room_tools, "opencode", "sudo-native-fixture").await
}

async fn native_fixture_for(room_tools: bool, adapter: &str, provider: &str) -> Fixture {
    let mut f = fixture_with_run_profile(None, room_tools, adapter, provider);
    let request = super::super::provider_reload::policy_reload_launch_request(
        &f.run,
        f.request.target_agent_id.as_deref().unwrap(),
        Default::default(),
    )
    .with_client_interface(ProviderClientInterface::NativeTui);
    let request = f
        .state
        .prepare_provider_launch_request_with_vault(request, "native catalog fixture")
        .await
        .unwrap();
    let mut run = RuntimeProviderRun::new(
        f.run.id(),
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: f.run.working_directory().map(ToOwned::to_owned),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    f.state
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    f.state.owned.provider_run_projection.update(run.clone());
    f.run = run;
    f
}

async fn approve(f: &Fixture) -> tokio::task::JoinHandle<Result<LocalDaemonResponse, DaemonError>> {
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
    task
}

#[tokio::test]
async fn sudo_native_first_turn_waits_for_deferred_catalog_refresh() {
    for (busy, refreshing) in [(false, false), (true, false), (false, true)] {
        let f = native_fixture(true).await;
        let session = f
            .state
            .owned
            .session_store
            .get_session(&f.request.session_id)
            .unwrap();
        let agent = f.request.target_agent_id.as_deref().unwrap();
        let changes = f
            .state
            .owned
            .provider_run_projection
            .catalog_changes()
            .clone();
        let mut watch = changes.subscribe(f.run.id()).unwrap();
        let lane = (!refreshing).then(|| {
            f.state
                .owned
                .provider_store
                .run_operation_lanes()
                .try_acquire(f.run.id())
                .unwrap()
        });
        let refresh = refreshing.then(|| changes.begin_refresh(f.run.id()).unwrap());
        assert_eq!(
            f.state
                .refresh_native_runtime_catalog(f.run.clone())
                .await
                .unwrap(),
            super::super::provider_reload::ProviderReloadOutcome::Deferred
        );
        let initial = watch.current().desired;
        if busy {
            f.state
                .owned
                .submit_local_prepared_prompt_with_queue_policy(
                    &crate::app::KernelPreparedPromptSubmission {
                        session_id: session.id().into(),
                        prompt: PromptQueueItem::new(
                            "busy-before-native-refresh",
                            &f.request.attachment_id,
                            agent,
                            "ordinary",
                            PromptStatus::Queued,
                        ),
                        force_queue: false,
                        refresh_projection: true,
                    },
                    false,
                )
                .unwrap()
                .unwrap();
        }
        let task = approve(&f).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        if busy {
            assert!(f
                .state
                .owned
                .prompt_state_owner
                .sudo_work_held(&session, agent));
            f.state
                .owned
                .prompt_state_owner
                .cancel_active_prompt_only(&session, agent)
                .unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(!task.is_finished(), "idle does not imply catalog readiness");
        assert!(f.state.list_sudo_turns("local")[0].prompt_id.is_none());
        assert!(f
            .state
            .owned
            .prompt_state_owner
            .sudo_work_held(&session, agent));
        assert_eq!(watch.current().desired, initial);
        drop(lane);
        drop(refresh);
        // Supplementary fixture acknowledgement of the normal tools/list seam.
        // No official harness or user-facing acceptance is claimed here.
        tokio::time::timeout(Duration::from_secs(5), async {
            while watch.current().desired == initial {
                watch.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        changes.observed(f.run.id(), changes.revision(f.run.id()).unwrap());
        let response = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            response,
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                ..
            }
        ));
        let window = f.state.list_sudo_turns("local").pop().unwrap();
        assert_eq!(window.provider_run_id.as_deref(), Some(f.run.id()));
        f.state
            .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
            .unwrap();
    }
}

// MP-08/MP-10/MP-11 A04: listing a sudo tool never arms a catalog continuation.
// The first turn refreshes explicitly; later window changes must not replay
// ordinary prompts when another catalog hint arrives.
#[tokio::test]
async fn sudo_window_never_changes_the_catalog_change_signature() {
    let f = fixture_with_options(None, true);
    let agent = f
        .state
        .owned
        .agent_store
        .get_agent(f.request.target_agent_id.as_deref().unwrap())
        .unwrap();
    let checkpoint = |name: &'static str| {
        let listed = f
            .router
            .runtime_tool_specs_for_auth_token("sudo-fixture-bearer")
            .iter()
            .any(|spec| spec.name == "chariox_kernel_request");
        (
            name,
            listed,
            f.state.runtime_catalog_signature_for_agent(&agent),
        )
    };
    let mut seen = vec![checkpoint("before")];
    let state = f.state.clone();
    let request = f.request.clone();
    let pending = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    popup(&f.state).await;
    seen.push(checkpoint("pending"));
    f.state
        .revoke_sudo(Some("local"), None, "fixture_cleanup")
        .unwrap();
    assert!(pending.await.unwrap().is_err());
    seen.push(checkpoint("pending revoked"));
    let turn = running(&f);
    seen.push(checkpoint("bound"));
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "fixture_cleanup")
        .unwrap();
    seen.push(checkpoint("ended"));
    let listed: Vec<_> = seen
        .iter()
        .map(|(name, listed, _)| (*name, *listed))
        .collect();
    assert_eq!(
        listed,
        [
            ("before", false),
            ("pending", false),
            ("pending revoked", false),
            ("bound", true),
            ("ended", false)
        ]
    );
    for (name, _, signature) in &seen {
        assert_eq!(signature, &seen[0].2, "{name}: catalog signature changed");
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Native {
        room_tools: bool,
    },
    /// Native Claude whose launch credential is held in the locked vault.
    VaultClaude,
    Relaunch,
}

#[derive(Clone, Copy)]
enum Step {
    Approve,
    HoldLane,
    ReleaseLane,
    Busy,
    Idle,
    Sleep(u64),
    LockVault,
    AnswerVault,
    Revoke,
    /// Busy -> approve -> idle until the Reloaded branch terminates the run.
    QueuedReload,
    BreakRelaunch,
    AckCatalog,
    AssertStableCatalog,
}

// MP-08/MP-10/MP-11: the first turn's 60 s catalog budget counts only idle
// refresh attempts: never busy work, never a human popup. Every failure ends
// the window with a typed error and leaves no popup, hold or timer behind.
// Native refresh runs on the tokio clock, so every row runs on paused time.
#[tokio::test(start_paused = true)]
async fn sudo_catalog_readiness_budget_counts_only_the_refresh() {
    crate::test_support::isolated_env_test!();
    use Step::*;
    let rows: [(&str, Kind, &'static [Step], Option<&str>); 11] = [
        (
            "sudo visibility never arms an ordinary prompt continuation",
            Kind::Native { room_tools: true },
            &[HoldLane, Busy, Approve, AssertStableCatalog, Idle, ReleaseLane, AckCatalog],
            None,
        ),
        ("idle refresh ok", Kind::Native { room_tools: true }, &[Approve, AckCatalog], None),
        (
            "idle refresh fails",
            Kind::Native { room_tools: true },
            &[Approve, Sleep(31)],
            Some("provider catalog refresh failed: provider did not fetch the changed runtime catalog"),
        ),
        (
            "idle contention exhausts the budget",
            Kind::Native { room_tools: true },
            &[HoldLane, Approve, Sleep(61)],
            Some("provider catalog refresh failed: not ready within 60 seconds"),
        ),
        (
            "busy then idle",
            Kind::Native { room_tools: false },
            &[HoldLane, Approve, Sleep(30), Busy, Sleep(65), Idle, Sleep(40), ReleaseLane, AckCatalog],
            None,
        ),
        (
            "vault popup answered after 2 min",
            Kind::VaultClaude,
            &[Busy, Approve, LockVault, Idle, Sleep(120), AnswerVault, AckCatalog],
            None,
        ),
        (
            "vault popup ignored until it expires",
            Kind::VaultClaude,
            &[Busy, Approve, LockVault, Idle, Sleep(301)],
            Some("provider catalog refresh failed: Chariox vault unlock was cancelled"),
        ),
        (
            "vault relocks between retries",
            Kind::VaultClaude,
            &[HoldLane, Approve, Sleep(1), LockVault, Sleep(1), AnswerVault, ReleaseLane, AckCatalog],
            None,
        ),
        (
            "vault relock answered after the remaining budget",
            Kind::VaultClaude,
            &[HoldLane, Approve, Sleep(1), LockVault, Sleep(61), AnswerVault, ReleaseLane, AckCatalog],
            None,
        ),
        (
            "window revoked while the vault popup is open",
            Kind::VaultClaude,
            &[Busy, Approve, LockVault, Idle, Sleep(120), Revoke, Sleep(1)],
            Some("queued sudo was revoked"),
        ),
        (
            "relaunch failure",
            Kind::Relaunch,
            &[QueuedReload, BreakRelaunch, Sleep(65)],
            Some("provider relaunch failed: not ready within 60 seconds"),
        ),
    ];
    let mut failed = vec![];
    for (name, kind, steps, failure) in rows {
        // A panicking row is reported by name; the other rows still run.
        if tokio::spawn(catalog_row(name, kind, steps, failure))
            .await
            .is_err()
        {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "failed rows: {failed:?}");
}

async fn catalog_row(
    name: &'static str,
    kind: Kind,
    steps: &'static [Step],
    failure: Option<&'static str>,
) {
    use Step::*;
    let f = match kind {
        Kind::Native { room_tools } => native_fixture(room_tools).await,
        Kind::VaultClaude => vault_claude_fixture().await,
        Kind::Relaunch => fixture_with_catalog_reload(),
    };
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let agent = f.request.target_agent_id.clone().unwrap();
    let changes = f
        .state
        .owned
        .provider_run_projection
        .catalog_changes()
        .clone();
    let mut watch = changes.subscribe(f.run.id());
    let initial = watch.as_ref().map(|watch| watch.current().desired);
    let catalog_agent = f.state.owned.agent_store.get_agent(&agent).unwrap();
    let initial_signature = f.state.runtime_catalog_signature_for_agent(&catalog_agent);
    let mut lane = None;
    let mut task = None;
    for step in steps {
        match *step {
            Approve => task = Some(approve(&f).await),
            HoldLane => {
                lane = f
                    .state
                    .owned
                    .provider_store
                    .run_operation_lanes()
                    .try_acquire(f.run.id())
            }
            ReleaseLane => lane = None,
            Busy => {
                f.state
                    .owned
                    .submit_local_prepared_prompt_with_queue_policy(
                        &crate::app::KernelPreparedPromptSubmission {
                            session_id: session.id().into(),
                            prompt: PromptQueueItem::new(
                                "ordinary-work",
                                &f.request.attachment_id,
                                &agent,
                                "ordinary",
                                PromptStatus::Queued,
                            ),
                            force_queue: false,
                            refresh_projection: true,
                        },
                        false,
                    )
                    .unwrap()
                    .unwrap();
            }
            Idle => {
                f.state
                    .owned
                    .prompt_state_owner
                    .cancel_active_prompt_only(&session, &agent)
                    .unwrap();
            }
            Sleep(seconds) => tokio::time::sleep(Duration::from_secs(seconds)).await,
            LockVault => {
                let vault = f.state.owned.config_projection.snapshot().user_config;
                crate::secret::lock_chariox_encrypted_vault(&vault.credential_vault.path).unwrap();
                crate::secret::clear_vault_secret_process_cache().unwrap();
            }
            AnswerVault => {
                for (title, choice, reply) in [
                    ("Unlock Chariox Vault", "passphrase", Some(PASSKEY)),
                    ("Choose Vault Unlock Duration", "unlock_operation", None),
                ] {
                    let open = vault_popup(&f, &agent).await;
                    assert_eq!(open.title(), Some(title), "{name}");
                    f.state
                        .resolve_terminal_runtime_interaction(
                            session.id(),
                            open.id(),
                            choice,
                            reply,
                            Some("local"),
                        )
                        .await
                        .unwrap();
                }
            }
            Revoke => {
                let window = f.state.list_sudo_turns("local").pop().unwrap();
                f.state
                    .revoke_sudo(Some("local"), Some(&window.entry_id), "owner_revoked")
                    .unwrap();
            }
            QueuedReload => {
                task = Some(queued_reload(&f).await.1);
            }
            // Its prepared cwd disappears during the policy relaunch delay.
            BreakRelaunch => std::fs::remove_dir_all(f.run.working_directory().unwrap()).unwrap(),
            AssertStableCatalog => {
                // Native launch preparation allocates a fresh MCP bearer;
                // wait for the authorization future to open its window.
                let token = f.run.runtime_mcp_auth_token().unwrap();
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !f.state.sudo_window_open_for_auth_token(token) {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("approved window should list its tool");
                assert_eq!(
                    f.state.runtime_catalog_signature_for_agent(&catalog_agent),
                    initial_signature,
                    "{name}: sudo visibility changed the catalog signature"
                );
                f.state
                    .runtime_catalog_registration_changed(&catalog_agent, &initial_signature);
                assert!(
                    !f.state
                        .owned
                        .pending_mcp_continuations
                        .write()
                        .contains_key(&agent),
                    "{name}: ordinary prompt continuation armed"
                );
            }
            AckCatalog => {
                // Supplementary tools/list acknowledgement; no live claim.
                tokio::time::timeout(Duration::from_secs(5), async {
                    let watch = watch.as_mut().unwrap();
                    while Some(watch.current().desired) == initial {
                        watch.changed().await.unwrap();
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{name}: refresh never started"));
                changes.observed(f.run.id(), changes.revision(f.run.id()).unwrap());
            }
        }
        if let Some(task) = &task {
            assert!(
                !task.is_finished() || step_is_last(steps, step),
                "{name}: finished early at a non-final step"
            );
        }
    }
    drop(lane);
    let task = task.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap_or_else(|_| panic!("{name}: sudo request still waiting"))
        .unwrap();
    match failure {
        None => {
            assert!(
                matches!(
                    result,
                    Ok(LocalDaemonResponse::PromptSubmitted {
                        outcome: PromptSubmissionOutcome::Started { .. },
                        ..
                    })
                ),
                "{name}: {result:?}"
            );
            let window = f.state.list_sudo_turns("local").pop().unwrap();
            assert_eq!(
                window.provider_run_id.as_deref(),
                Some(f.run.id()),
                "{name}"
            );
            f.state
                .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
                .unwrap();
        }
        Some(expected) => {
            assert!(
                matches!(&result, Err(DaemonError::LocalTransport {
                        operation: "kernel access", message
                    }) if message.contains(expected)),
                "{name}: {result:?}"
            );
            assert!(f.state.list_sudo_turns("local").is_empty(), "{name}");
            assert!(
                !f.state
                    .owned
                    .prompt_state_owner
                    .sudo_work_held(&session, &agent),
                "{name}"
            );
            let snapshot = f.state.owned.session_snapshot(session.id()).unwrap();
            assert!(snapshot.sudo_windows().is_empty(), "{name}");
            assert!(
                f.state.owned.sudo_timers.lock().unwrap().is_empty(),
                "{name}"
            );
            assert!(
                f.state.owned.sudo_scopes.lock().unwrap().is_empty(),
                "{name}"
            );
        }
    }
    // No popup may outlive the request: an open vault popup would
    // silently discard a passphrase typed after the window ended.
    let open = f
        .state
        .owned
        .session_store
        .get_session(session.id())
        .unwrap()
        .active_interactions()
        .iter()
        .map(|i| i.title().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert!(open.is_empty(), "{name}: popups left open: {open:?}");
    assert!(f.state.passkey_prompts_for("local").is_empty(), "{name}");
}

fn step_is_last(steps: &[Step], step: &Step) -> bool {
    std::ptr::eq(steps.last().unwrap(), step)
}

async fn vault_popup(f: &Fixture, agent: &str) -> crate::session::RuntimeInteraction {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(open) = f
                .state
                .owned
                .session_store
                .get_session(&f.request.session_id)
                .unwrap()
                .active_interaction_for_agent(agent)
                .filter(|i| i.id().starts_with("vault-"))
            {
                return open.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("vault popup should be open")
}

/// Native Claude with a Chariox-vault launch credential (isolated home only).
async fn vault_claude_fixture() -> Fixture {
    let f = native_fixture_for(true, "claude", "claude").await;
    let config = f.state.owned.config_projection.snapshot();
    let profile = f.run.account_profile().to_owned();
    crate::secret::unlock_chariox_encrypted_vault(
        &config.user_config.credential_vault.path,
        PASSKEY,
        crate::secret::VaultUnlockLease::Operation,
    )
    .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        "local",
        "claude",
        &profile,
        "sudo-catalog-fixture-token",
        false,
    )
    .unwrap();
    f
}

// MP-08/MP-10/MP-11 R947-2: worker acknowledgement waits for native tools/list.
#[tokio::test]
async fn leased_sudo_waits_for_deferred_catalog_refresh() {
    for (busy, refreshing, interrupted) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let f = fixture_with_options(None, true);
        let leased = {
            let mut app = f.app.lock().await;
            let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
                .create_execution_lease("home", "room", "agent", false, "local")
                .unwrap();
            crate::app::RemoteLeaseRuntime::new(&mut app)
                .create_leased_agent_from_base_directory(
                    f.run.working_directory().unwrap(),
                    &lease.id,
                    "managed-dev-stub",
                    "default",
                    Some("controlled-cancel-idle".into()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
        };
        let request = crate::provider::LaunchProviderRequest::new(
            &leased.backing_session_id,
            "opencode",
            "sudo-native-fixture",
            "default",
            "default",
        )
        .with_agent_id(&leased.backing_agent_id)
        .with_client_interface(ProviderClientInterface::NativeTui);
        let request = f
            .state
            .prepare_provider_launch_request_with_vault(request, "leased native fixture")
            .await
            .unwrap();
        let mut run = RuntimeProviderRun::new(
            "leased-native-fixture",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "metadata-only".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: Some(f.run.working_directory().unwrap().to_owned()),
                structured_endpoint: None,
            },
        );
        run.mark_running();
        f.state
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        f.state.owned.provider_run_projection.update(run.clone());
        let changes = f
            .state
            .owned
            .provider_run_projection
            .catalog_changes()
            .clone();
        let mut watch = changes.subscribe(run.id()).unwrap();
        let lane = (!refreshing).then(|| {
            f.state
                .owned
                .provider_store
                .run_operation_lanes()
                .try_acquire(run.id())
                .unwrap()
        });
        let refresh = refreshing.then(|| changes.begin_refresh(run.id()).unwrap());
        let initial = watch.current().desired;
        let session = f
            .state
            .owned
            .session_store
            .get_session(&leased.backing_session_id)
            .unwrap();
        if busy {
            let attachment = {
                let mut app = f.app.lock().await;
                crate::app::KernelSessionService::new(&mut app)
                    .attach(crate::attachment::AttachRequest::new(
                        session.id(),
                        "leased-busy-fixture",
                        crate::attachment::ClientCapabilityLevel::FullTerminal,
                    ))
                    .unwrap()
            };
            f.state
                .owned
                .submit_local_prepared_prompt_with_queue_policy(
                    &crate::app::KernelPreparedPromptSubmission {
                        session_id: session.id().into(),
                        prompt: PromptQueueItem::new(
                            "busy-before-leased-refresh",
                            attachment.id(),
                            &leased.backing_agent_id,
                            "ordinary",
                            PromptStatus::Queued,
                        ),
                        force_queue: false,
                        refresh_projection: true,
                    },
                    false,
                )
                .unwrap()
                .unwrap();
        }
        let state = f.state.clone();
        let id = leased.id.clone();
        let mut task = tokio::spawn(async move {
            state
                .update_relay_leased_sudo(
                    &id,
                    "sudo-home-prompt",
                    crate::transport::relay_peer::LeasedSudoGrant {
                        entry_id: "leased-window".into(),
                        revision: 0,
                        remaining_ms: 60_000,
                        initial: true,
                    },
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !task.is_finished(),
            "worker acknowledged sudo before the deferred catalog refreshed"
        );
        assert_eq!(watch.current().desired, initial);
        if interrupted {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            let state = f.state.clone();
            let id = leased.id.clone();
            task = tokio::spawn(async move {
                state
                    .update_relay_leased_sudo(
                        &id,
                        "sudo-home-prompt",
                        crate::transport::relay_peer::LeasedSudoGrant {
                            entry_id: "leased-window".into(),
                            revision: 1,
                            remaining_ms: 60_000,
                            initial: false,
                        },
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(
                !task.is_finished(),
                "same-entry renewal skipped the interrupted refresh"
            );
            assert_eq!(watch.current().desired, initial);
        }
        if busy {
            f.state
                .owned
                .prompt_state_owner
                .cancel_active_prompt_only(&session, &leased.backing_agent_id)
                .unwrap();
        }
        drop(lane);
        drop(refresh);
        tokio::time::timeout(Duration::from_secs(5), async {
            while watch.current().desired == initial {
                watch.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(
            !task.is_finished(),
            "catalog invalidation alone is not readiness"
        );
        changes.observed(run.id(), changes.revision(run.id()).unwrap());
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            f.state
                .owned
                .provider_store
                .get_run_for_agent(session.id(), &leased.backing_agent_id)
                .unwrap()
                .id(),
            run.id()
        );
        f.state
            .update_relay_leased_sudo(
                &leased.id,
                "sudo-home-prompt",
                crate::transport::relay_peer::LeasedSudoGrant {
                    entry_id: "leased-window".into(),
                    revision: u64::from(interrupted),
                    remaining_ms: 0,
                    initial: false,
                },
            )
            .await
            .unwrap();
    }
}
