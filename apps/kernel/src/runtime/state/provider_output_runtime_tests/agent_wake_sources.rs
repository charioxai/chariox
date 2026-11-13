//! MP-08 / MP-09 / MP-10 / MP-11 A02/A03: the wake source interface against
//! a real runtime state, and the `chariox.events.*` tools through the runtime
//! MCP entry. Regression coverage, not acceptance.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, AgentWake, ExecutionState, Operation, Outcome, WakeEnd,
};
use crate::runtime::state::wake_driver::{
    Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence, WakeSink,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;

const KIND: &str = "test-source";

struct TestDriver {
    sink: WakeSink,
    armed: StdMutex<Vec<String>>,
    fail_arm: AtomicBool,
    cancel_pending: AtomicBool,
    lost: AtomicBool,
    /// Holds the next liveness probe until released, as a slow source would.
    stall: StdMutex<
        Option<(
            std::sync::mpsc::SyncSender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}

impl WakeDriver for TestDriver {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError> {
        if self.fail_arm.load(Ordering::Acquire) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "test source refused to start",
            ));
        }
        self.armed.lock().unwrap().push(wake.id.clone());
        self.sink.verified(&wake.id)
    }
    fn cancel(&self, _wake: &AgentWake) -> Result<Cancel, DaemonError> {
        Ok(if self.cancel_pending.load(Ordering::Acquire) {
            Cancel::Pending
        } else {
            Cancel::Done
        })
    }
    fn recover(&self, _wake: &AgentWake) -> Recovery {
        Recovery::Lost("test source does not survive a restart".into())
    }
    fn liveness(&self, _wake: &AgentWake) -> Liveness {
        if let Some((entered, release)) = self.stall.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = release.recv();
        }
        if self.lost.load(Ordering::Acquire) {
            Liveness::Lost("test source vanished".into())
        } else {
            Liveness::Live
        }
    }
}

struct Fixture {
    runtime: KernelRuntimeState,
    driver: Arc<TestDriver>,
    room: String,
    agent: String,
    run: String,
    prompt_id: String,
    app: Arc<Mutex<DaemonApp>>,
    _pty: InertPtyCleanup,
    _worktree: crate::test_support::TestWorktree,
}

/// `running_prompt` leaves a real turn running for the agent, as a provider
/// run has while it calls tools; without it the agent is idle.
async fn fixture(name: &str, running_prompt: bool) -> Fixture {
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .unwrap();
    let prompt_id = if running_prompt {
        app.update_provider_run_projection(run.clone());
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-wake-sources",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "wait for the test source",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("first prompt starts immediately");
        };
        prompt.id().to_string()
    } else {
        String::new()
    };
    spawn_inert_pty_for_run(&mut app, run.id());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let driver = Arc::new(TestDriver {
        sink: runtime.wake_sink(KIND),
        armed: Default::default(),
        fail_arm: AtomicBool::new(false),
        cancel_pending: AtomicBool::new(false),
        lost: AtomicBool::new(false),
        stall: Default::default(),
    });
    runtime.register_wake_driver(Arc::downgrade(&driver) as std::sync::Weak<dyn WakeDriver>);
    Fixture {
        runtime,
        driver,
        room: session.id().into(),
        agent: agent.id().into(),
        run: run.id().into(),
        prompt_id,
        _pty: InertPtyCleanup {
            app: app.clone(),
            provider_run_id: run.id().into(),
        },
        app,
        _worktree: worktree,
    }
}

impl Fixture {
    fn store(&self) -> &crate::durable_state::DurableKernelStateStore {
        &self.runtime.owned.durable_state_store
    }
    /// Starts a task for `prompt` the way prompt dispatch does.
    fn begin(&self, prompt: &str) -> AgentTaskExecution {
        let Outcome::Task(task) = self
            .store()
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: self.agent.clone(),
                prompt: prompt.into(),
                run: Some(self.run.clone()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("begin returns the task");
        };
        task
    }
    fn arm(&self, task: &AgentTaskExecution, id: &str) -> Result<AgentWake, DaemonError> {
        let mut wake = self.wake(task, id);
        wake.state = "starting".into();
        self.runtime.arm_wake_source(task, &wake)?;
        wake = self.store().agent_wake(id).unwrap().unwrap();
        Ok(wake)
    }
    fn wake(&self, task: &AgentTaskExecution, id: &str) -> AgentWake {
        serde_json::from_value(serde_json::json!({
            "id": id, "task_id": task.task_id, "room_id": self.room, "agent_id": self.agent,
            "registration_id": format!("completion-{id}"), "kind": KIND, "label": "test source",
            "state": "starting", "created_at_ms": crate::session::unix_epoch_ms(),
            "verified_at_ms": null, "next_due_ms": null, "interval_ms": null,
            "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
            "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
            "last_sequence": null, "last_delivery": null, "last_delivered_at_ms": null,
            "last_acknowledged_at_ms": null, "alerted_sequence": null
        }))
        .unwrap()
    }
    fn wait_on(&self, task: &AgentTaskExecution, registration: &str) {
        let now = crate::session::unix_epoch_ms();
        self.store()
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                registrations: vec![registration.into()],
                cursor: 0,
                deadline: Some(now + 60_000),
                reason: "Wait for the test source".into(),
                now,
            })
            .unwrap();
        self.store()
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: self.agent.clone(),
                prompt: task.prompt_id.clone(),
                run: self.run.clone(),
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap();
    }
    fn task(&self, id: &str) -> AgentTaskExecution {
        self.store()
            .agent_tasks(Some(&self.room), Some(&self.agent))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == id)
            .unwrap()
    }
    fn wake_state(&self, id: &str) -> String {
        self.store().agent_wake(id).unwrap().unwrap().state
    }
    fn occurrence(&self, wake: &str, id: &str, end: Option<WakeEnd>) -> WakeOccurrence {
        WakeOccurrence {
            wake_id: wake.into(),
            occurrence_id: id.into(),
            kind: "output_matched".into(),
            payload: serde_json::json!({"line": "ready"}),
            summary: Some("matched".into()),
            end,
        }
    }
    fn notices(&self) -> String {
        self.runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&self.room, Some(&self.agent))
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == crate::history::SessionHistoryEntryKind::Notice)
            .map(|e| e.text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// A fixture kernel must end with its test. Its open decisions stay in the process-wide
// interaction store while it lives, so a later test kernel raising the same `prompt-1`
// decision was refused as already pending.
#[test]
fn a_fixture_kernel_ends_with_its_test() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let kernel = runtime.block_on(async {
        let f = fixture("fixture-kernel-lifetime", true).await;
        f.runtime.owned.session_store.weak_identity()
    });
    drop(runtime);
    assert_eq!(
        kernel.strong_count(),
        0,
        "the fixture kernel outlived its test"
    );
}

#[tokio::test]
async fn ev13_driver_occurrence_wakes_the_waiting_task_once() {
    let f = fixture("ev13-driver-occurrence", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    assert_eq!(
        wake.state, "running",
        "the driver's receipt is proof of life"
    );
    assert!(wake.verified_at_ms.is_some());
    assert_eq!(f.driver.armed.lock().unwrap().as_slice(), ["wake-a"]);
    f.wait_on(&task, &wake.registration_id);
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);

    let sink = f.runtime.wake_sink(KIND);
    sink.occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    sink.occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    // The sink schedules the delivery itself; the item leaves `pending`
    // when the idle agent has accepted the wake turn.
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if events.iter().all(|e| e.state == "accepted") && !events.is_empty() {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "wake was not delivered: {:?} queued={:?} active={:?}",
            f.store().agent_inbox(&f.room, &f.agent, 0).unwrap(),
            f.runtime
                .owned
                .session_store
                .get_session(&f.room)
                .unwrap()
                .queued_prompts_for_agent(&f.agent)
                .map(|q| q
                    .iter()
                    .map(|p| (p.id().to_string(), format!("{:?}", p.status())))
                    .collect::<Vec<_>>()),
            f.runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &f.runtime.owned.session_store.get_session(&f.room).unwrap(),
                    &f.agent
                )
                .map(|p| (p.id().to_string(), format!("{:?}", p.status())))
        )
    });
    assert_eq!(events.len(), 1, "a replayed occurrence is one admission");
    assert_eq!(events[0].state, "accepted");
    let woken = f.task(&task.task_id);
    assert_eq!(woken.state, ExecutionState::Working);
    assert_eq!(woken.prompt_id, events[0].prompt_id.clone().unwrap());
    assert_eq!(
        f.wake_state("wake-a"),
        "running",
        "the source keeps running"
    );

    sink.occurred(f.occurrence("wake-a", "exit", Some(WakeEnd::Succeeded)))
        .unwrap();
    assert_eq!(f.wake_state("wake-a"), "exited");
}

// MP-08 / MP-09 / MP-10 / MP-11: a worker can write a steer before its ACK reaches home.
#[tokio::test]
async fn delayed_leased_steer_ack_keeps_the_original_receipt_deadline_while_the_agent_is_busy() {
    let home = fixture("pcd11-delayed-leased-steer-ack", true).await;
    home.begin(&home.prompt_id);
    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.accept_remote_leases = true;
    let mut worker = DaemonApp::bootstrap(worker_config.clone()).unwrap();
    let home_config = home.runtime.owned.config_projection.snapshot();
    let owner = home
        .runtime
        .owned
        .session_store
        .get_session(&home.room)
        .unwrap()
        .owner_user_id()
        .to_owned();
    let lease = crate::app::RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(
            &home_config.daemon_id,
            &home.room,
            &home.agent,
            false,
            &owner,
        )
        .unwrap();
    let leased = crate::app::RemoteLeaseRuntime::new(&mut worker)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("default".into()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: home.room.clone(),
        home_agent_id: home.agent.clone(),
        home_prompt_id: home.prompt_id.clone(),
        home_turn_id: home.prompt_id.clone(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(crate::session::PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "active leased turn".into(),
    };
    let (worker_run, _) = crate::app::RemoteLeaseRuntime::new(&mut worker)
        .submit_leased_prompt_with_workflow_context(
            &leased.id,
            "active leased turn",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    spawn_inert_pty_for_run(&mut worker, &worker_run);
    let worker = Arc::new(Mutex::new(worker));
    let _worker_pty = InertPtyCleanup {
        app: worker.clone(),
        provider_run_id: worker_run.clone(),
    };
    let worker_router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(worker.clone(), 1);
    home.runtime
        .owned
        .agent_store
        .bind_remote_execution(
            &home.agent,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker_config.daemon_id,
                worker_machine_id: worker_config.host_machine_id,
                execution_lease_id: lease.id,
                leased_agent_id: leased.id.clone(),
                active_worker_provider_run_id: Some(worker_run.clone()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .unwrap();
    let mut event = crate::durable_state::agent_lifecycle::occurrence(
        &home.room,
        &home.agent,
        "source",
        "late-ack",
        "message",
        serde_json::json!({"text":"PCD11_STEER_WRITTEN_ONCE"}),
    );
    event.urgent = true;
    let Outcome::Event(event) = home
        .store()
        .agent_lifecycle(Operation::Occur(event))
        .unwrap()
    else {
        panic!("event");
    };
    let steer_id = format!("agent-event-{}-{}", home.agent, event.sequence);
    let at = crate::session::unix_epoch_ms();
    home.store()
        .agent_lifecycle(Operation::Attempt {
            room: home.room.clone(),
            agent: home.agent.clone(),
            sequence: event.sequence,
            prompt: steer_id.clone(),
            target: Some(home.prompt_id.clone()),
            run: Some(crate::provider::projected_leased_provider_run_id(
                &leased.id,
                &worker_run,
            )),
            now: at,
            work: None,
            through: 0,
        })
        .unwrap();
    // Drive the real worker entry and retain its ACK before recording it at home.
    let ack = worker_router
        .relay_steer_leased_prompt(
            &leased.id,
            &steer_id,
            &home.prompt_id,
            "PCD11_STEER_WRITTEN_ONCE",
            "",
            Vec::new(),
            None,
        )
        .await
        .unwrap();
    assert!(!ack.1);
    let writes = || {
        worker
            .try_lock()
            .unwrap()
            .terminal()
            .input_records()
            .into_iter()
            .filter(|record| {
                String::from_utf8_lossy(&record.bytes).contains("PCD11_STEER_WRITTEN_ONCE")
            })
            .count()
    };
    assert_eq!(
        writes(),
        1,
        "worker provider I/O happened before the home receipt"
    );
    let before = home
        .store()
        .agent_inbox(&home.room, &home.agent, 0)
        .unwrap()
        .remove(0);
    assert!(
        before.submit_epoch.is_none(),
        "leased I/O is not yet represented by a home epoch"
    );
    assert!(home
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &home
                .runtime
                .owned
                .session_store
                .get_session(&home.room)
                .unwrap(),
            &home.agent
        )
        .is_some());
    assert!(!home
        .runtime
        .agent_inbox_waits_for_vault(&home.room, &home.agent));
    home.store()
        .agent_lifecycle(Operation::Sweep {
            now: at + crate::durable_state::agent_lifecycle::DELIVERY_TIMEOUT_MS + 1,
            vault_wait_recipients: Vec::new(),
            busy_recipients: vec![(home.room.clone(), home.agent.clone())],
            held_work: Vec::new(),
        })
        .unwrap();
    let timed_out = home
        .store()
        .agent_inbox(&home.room, &home.agent, 0)
        .unwrap()
        .remove(0);
    assert_eq!(
        timed_out.state, "blocked",
        "an active leased turn must not renew an already-sent receipt window"
    );
    assert_eq!(timed_out.attempted_at_ms, Some(at));
    home.runtime
        .record_agent_delivery_receipt(&home.room, &home.agent, event.sequence, "accepted")
        .unwrap();
    let replay = worker_router
        .relay_steer_leased_prompt(
            &leased.id,
            &steer_id,
            &home.prompt_id,
            "PCD11_STEER_WRITTEN_ONCE",
            "",
            Vec::new(),
            None,
        )
        .await
        .unwrap();
    assert!(replay.1);
    assert_eq!(
        writes(),
        1,
        "late acceptance reconciliation never repeats provider I/O"
    );
    let after = home
        .store()
        .agent_inbox(&home.room, &home.agent, 0)
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].sequence, event.sequence);
    assert_eq!(after[0].state, "accepted");
}

// MP-08 / MP-09 / MP-10 / MP-11 PCD-11: a cold provider cannot consume an answer before Vault approval.
#[tokio::test]
async fn declined_vault_start_blocks_the_wake_with_its_reason_and_resume_retries_the_answer_once() {
    crate::test_support::isolated_env_test!();
    refused_vault_wake_start(false).await;
}

#[tokio::test]
async fn timed_out_vault_start_blocks_the_wake_with_its_reason_and_resume_retries_the_answer_once()
{
    crate::test_support::isolated_env_test!();
    refused_vault_wake_start(true).await;
}

async fn refused_vault_wake_start(timed_out: bool) {
    let _env = crate::env_lock::lock();
    let private = crate::test_support::TestWorktree::new("pcd11-vault-refused-state");
    std::env::set_var("CHARIOX_HOME", private.path());
    #[cfg(unix)]
    let _claude = crate::test_support::TestClaudeExecutable::new();
    let f = fixture("pcd11-vault-refused-wake", true).await;
    let vault = private.path().join("credentials.vault");
    let mut config = f.runtime.owned.config_projection.snapshot();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    f.runtime.owned.config_projection.update(config.clone());
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, "test passphrase").unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &vault,
        "test passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let registry = f.app.lock().await.provider_account_profile_registry();
    let profile = registry
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            "PCD11 vaulted fixture",
        )
        .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &profile.profile_id,
        "synthetic-setup-token",
        false,
    )
    .unwrap();
    crate::test_support::authenticate_provider_account(
        &registry,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &profile.profile_id,
    )
    .unwrap();
    let task = f.begin(&f.prompt_id);
    let question = crate::session::RuntimeInteraction::new(
        "pcd11-original-question",
        &f.agent,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Continue after restart?",
        vec![crate::session::RuntimeInteractionChoice::new(
            "done", "Done", "done", None,
        )],
        None,
        Some(900),
        None,
    );
    f.runtime
        .arm_interaction_wake(&f.room, &f.agent, question)
        .unwrap();
    let wake = f
        .store()
        .agent_wake("pcd11-original-question")
        .unwrap()
        .unwrap();
    f.wait_on(&task, &wake.registration_id);
    {
        let mut app = f.app.lock().await;
        app.prompt_owner_complete_active_prompt_only(&f.room, &f.agent)
            .unwrap();
        app.pty_mut().remove_process(&f.run).unwrap();
        app.providers_mut()
            .terminate_run_provider_only(&f.room, &f.run)
            .unwrap();
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                &f.agent,
                "claude",
                Some("claude-sonnet".into()),
                None,
                Some(profile.profile_id.clone()),
                crate::provider::ProviderResumeState::default(),
            )
            .unwrap();
    }
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    let owner = f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .owner_user_id()
        .to_owned();
    f.runtime
        .owned
        .resolve_runtime_interaction(
            &f.room,
            "pcd11-original-question",
            "done",
            None,
            Some(&owner),
            true,
        )
        .unwrap();
    let original = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .remove(0);
    let runtime = f.runtime.clone();
    let room = f.room.clone();
    let agent = f.agent.clone();
    let delivery = tokio::spawn(async move { runtime.deliver_agent_inbox(&room, &agent).await });
    let card = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
            if let Some(card) = session.active_interactions().iter().find(|card| {
                card.agent_id() == Some(f.agent.as_str())
                    && card
                        .choices()
                        .iter()
                        .any(|choice| choice.vault_unlock().is_some())
            }) {
                break card.id().to_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cold launch asks for Vault approval");
    let pending = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .remove(0);
    assert!(
        pending.submit_epoch.is_none(),
        "no provider I/O while the owner decides"
    );
    // The typed Vault card above proves an owner wait; advance only the ledger clock.
    let busy = vec![(f.room.clone(), f.agent.clone())];
    f.store()
        .agent_lifecycle(Operation::Sweep {
            now: pending.attempted_at_ms.unwrap()
                + crate::durable_state::agent_lifecycle::DELIVERY_TIMEOUT_MS
                + 1,
            vault_wait_recipients: busy,
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        })
        .unwrap();
    assert_eq!(
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()[0].state,
        "submitting",
        "a visible owner wait is not unconfirmed provider I/O"
    );
    if timed_out {
        f.runtime
            .timeout_runtime_interaction(&f.room, &card)
            .await
            .unwrap();
    } else {
        f.runtime
            .owned
            .resolve_runtime_interaction(&f.room, &card, "cancel", None, Some(&owner), true)
            .unwrap();
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(10), delivery)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let blocked = f.task(&task.task_id);
    assert_eq!(blocked.state, ExecutionState::Blocked);
    assert!(
        blocked.reason.contains(if timed_out {
            "Vault unlock timed out"
        } else {
            "Vault unlock was declined"
        }),
        "{}",
        blocked.reason
    );
    assert!(!blocked.reason.contains("Unconfirmed"));
    assert!(blocked.reason.contains("Resume"));
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    let retained = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained[0].state, "pending",
        "the failed start never discards the answer"
    );
    assert_eq!(retained[0].payload, original.payload);
    f.store()
        .agent_lifecycle(Operation::Sweep {
            now: crate::session::unix_epoch_ms()
                + 2 * crate::durable_state::agent_lifecycle::DELIVERY_TIMEOUT_MS,
            vault_wait_recipients: Vec::new(),
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        })
        .unwrap();
    assert_eq!(
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()[0].state,
        "pending",
        "a definitive refusal waits for Resume rather than aging into unconfirmed delivery"
    );
    assert_eq!(f.task(&task.task_id).reason, blocked.reason);

    let retry_run = {
        let mut app = f.app.lock().await;
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                &f.agent,
                "dev-stub",
                None,
                None,
                None,
                crate::provider::ProviderResumeState::default(),
            )
            .unwrap();
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    &f.room, "dev-stub", "dev-stub", "default", "sonnet",
                )
                .with_agent_id(&f.agent),
            )
            .unwrap();
        spawn_inert_pty_for_run(&mut app, run.id());
        run.id().to_owned()
    };
    let _retry_pty = InertPtyCleanup {
        app: f.app.clone(),
        provider_run_id: retry_run,
    };
    f.runtime
        .resolve_agent_task_owner_action(
            &blocked,
            super::super::agent_task_owner_resolution::OwnerTaskAction::Resume,
        )
        .await;
    let retried = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0].sequence, original.sequence);
    assert_eq!(
        retried[0].state, "accepted",
        "explicit Resume delivers the original answer once"
    );
    assert_eq!(retried[0].payload, original.payload);
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    assert_eq!(
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap().len(),
        1
    );
}

#[tokio::test]
async fn failed_wake_dispatch_settles_the_admitted_prompt_and_keeps_the_receipt() {
    let f = fixture("ev13-dispatch-failure", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);
    // The provider process is gone before the wake turn can reach it.
    f.app.lock().await.pty_mut().remove_process(&f.run).unwrap();

    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    // The attempt returns the event to pending with its receipt; the admitted
    // wake turn must not stay the active prompt of an agent no provider serves.
    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
            let active = f
                .runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &f.agent);
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if active.is_none()
                && events
                    .iter()
                    .any(|e| e.state == "pending" && e.attempted_at_ms.is_some())
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        settled.is_ok(),
        "a failed wake dispatch left its prompt active: {:?}",
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()
    );
}

#[tokio::test]
async fn refused_wake_after_dispatch_admission_is_retried_by_the_sweep() {
    let f = fixture("ev13-refused-wake-retry", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);
    let expected_wait = f.task(&task.task_id).wait.unwrap();
    f.store()
        .agent_lifecycle(Operation::WakeOccurred {
            id: wake.id.clone(),
            occurrence: "line-1".into(),
            kind: "output_matched".into(),
            payload: serde_json::json!({"line": "ready"}),
            summary: Some("matched".into()),
            end: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let event = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .remove(0);
    let prompt_id = format!("agent-event-{}-{}", f.agent, event.sequence);
    f.store()
        .agent_lifecycle(Operation::Attempt {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequence: event.sequence,
            prompt: prompt_id.clone(),
            target: None,
            run: None,
            work: None,
            now: crate::session::unix_epoch_ms(),
            through: 0,
        })
        .unwrap();
    let agent = f.runtime.owned.agent_store.get_agent(&f.agent).unwrap();
    let attachment = f
        .runtime
        .ensure_agent_message_attachment(&f.room, &agent)
        .unwrap();
    let submission = f
        .runtime
        .submit_prepared_prompt_with_queue_policy(
            crate::app::KernelPreparedPromptSubmission {
                session_id: f.room.clone(),
                prompt: crate::session::PromptQueueItem::new(
                    &prompt_id,
                    &attachment,
                    &f.agent,
                    "Kernel event inbox, untrusted data",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            },
            false,
        )
        .await
        .unwrap();
    let dispatch = submission.dispatch.unwrap();
    let permit = f.runtime.provider_runtime_lanes.acquire(&f.run).await;
    f.runtime
        .pump_owned_provider_output(&f.room, &f.run, Vec::new(), false)
        .await
        .unwrap();
    // A real transport can disappear after liveness and before obtaining its
    // writer. Enter the shared post-liveness dispatcher at that exact seam.
    f.app.lock().await.pty_mut().remove_process(&f.run).unwrap();
    let result = f
        .runtime
        .enqueue_prompt_dispatch_after_liveness_with_acceptance(&dispatch, &f.runtime.owned)
        .await;
    assert!(result.is_err(), "expected missing writer: {result:?}");
    let result = result.map(|acceptance| acceptance != crate::runtime::state::local_prompt_dispatch_runtime::PromptDispatchAcceptance::Dropped);
    let admitted = f.task(&task.task_id);
    assert_eq!(
        admitted.prompt_id, prompt_id,
        "dispatch must reach Begin(run: Some(...))"
    );
    assert_eq!(admitted.provider_run_id.as_deref(), Some(f.run.as_str()));
    assert!(f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()[0]
        .submit_epoch
        .is_none());
    f.runtime
        .record_agent_event_dispatch_result(&dispatch, &result)
        .unwrap();
    assert!(f
        .runtime
        .fail_prompt_dispatch(dispatch, result.unwrap_err())
        .await
        .is_err());
    drop(permit);

    let restored = f.task(&task.task_id);
    assert_eq!(restored.state, ExecutionState::Waiting);
    let restored_wait = restored.wait.unwrap();
    assert_eq!(
        restored_wait.registration_ids,
        expected_wait.registration_ids
    );
    assert_eq!(restored_wait.deadline_ms, expected_wait.deadline_ms);
    assert_eq!(restored_wait.inbox_cursor, expected_wait.inbox_cursor);
    assert_eq!(restored_wait.started_at_ms, expected_wait.started_at_ms);
    assert!(restored.pending_prompt_id.is_none());

    spawn_inert_pty_for_run(&mut *f.app.lock().await, &f.run);
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(events.len(), 1, "retry the original occurrence");
    assert_eq!(events[0].state, "accepted");
    assert!(events[0].submit_epoch.is_some());
    let retried = f.task(&task.task_id);
    assert_eq!(retried.state, ExecutionState::Working);
    assert!(retried.pending_prompt_id.is_none());
    assert!(
        retried.wait.is_none(),
        "the provider-I/O fence consumes the old wait"
    );
    f.wait_on(&retried, &wake.registration_id);
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
}

#[tokio::test]
async fn ev13_forged_source_identity_is_refused_before_the_ledger() {
    let f = fixture("ev13-forged-source", false).await;
    let task = f.begin("turn-1");
    f.arm(&task, "wake-a").unwrap();

    let timer_sink = f.runtime.wake_sink("timer");
    assert!(timer_sink.verified("wake-a").is_err());
    assert!(timer_sink
        .occurred(f.occurrence("wake-a", "forged", None))
        .is_err());

    let sink = f.runtime.wake_sink(KIND);
    for reserved in [
        "source_completed",
        "source_lost",
        "deadline_reached",
        "wait_recheck",
        "delivery_corrupt",
        "timer_fired",
    ] {
        let mut occurrence = f.occurrence("wake-a", reserved, None);
        occurrence.kind = reserved.into();
        assert!(
            sink.occurred(occurrence).is_err(),
            "a driver cannot report the ledger's own kind `{reserved}`"
        );
    }
    assert!(f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .is_empty());
    assert_eq!(f.wake_state("wake-a"), "running");
}

#[tokio::test]
async fn ev13_failed_arm_leaves_no_armed_wake() {
    let f = fixture("ev13-failed-arm", false).await;
    let task = f.begin("turn-1");
    f.driver.fail_arm.store(true, Ordering::Release);
    assert!(f.arm(&task, "wake-a").is_err());
    assert_eq!(f.wake_state("wake-a"), "cancelled");
    assert!(!f
        .task(&task.task_id)
        .obligations
        .iter()
        .any(|o| o.status == "open"));
    let unregistered = AgentWake {
        kind: "unregistered".into(),
        ..f.wake(&task, "wake-b")
    };
    assert!(f.runtime.arm_wake_source(&task, &unregistered).is_err());
    assert!(f.store().agent_wake("wake-b").unwrap().is_none());
}

#[tokio::test]
async fn ev13_cancel_done_and_pending() {
    let f = fixture("ev13-cancel", false).await;
    let task = f.begin("turn-1");
    let done = f.arm(&task, "wake-done").unwrap();
    f.runtime
        .cancel_wake_source(&done, &task.task_id, Some(&task.prompt_id))
        .unwrap();
    assert_eq!(f.wake_state("wake-done"), "cancelled");

    f.driver.cancel_pending.store(true, Ordering::Release);
    let pending = f.arm(&task, "wake-pending").unwrap();
    f.runtime
        .cancel_wake_source(&pending, &task.task_id, Some(&task.prompt_id))
        .unwrap();
    assert_eq!(
        f.wake_state("wake-pending"),
        "cancelling",
        "a stop request is not a stop"
    );
    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence("wake-pending", "stopped", Some(WakeEnd::Failed)))
        .unwrap();
    assert_ne!(f.wake_state("wake-pending"), "cancelling");
}

// D2: an ordinary turn whose settlement was lost (for example by a restart)
// ends as a stopped turn; it never asks the owner for a decision.
#[tokio::test]
async fn sweep_ends_a_stranded_plain_turn_without_asking_the_owner() {
    let f = fixture("sweep-plain-turn", false).await;
    let long_ago = crate::session::unix_epoch_ms() - 10 * 60 * 1000;
    f.store()
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: "stranded".into(),
            run: Some(f.run.clone()),
            now: long_ago,
        })
        .unwrap();

    f.runtime.sweep_agent_lifecycle().await.unwrap();

    let task = f
        .store()
        .agent_tasks(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == "stranded")
        .unwrap();
    assert_eq!(task.state, ExecutionState::Cancelled);
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(session.active_interactions().is_empty());
}

#[tokio::test]
async fn agent_deadline_stops_an_active_turn_and_fences_later_admission() {
    let f = fixture("agent-deadline-active-turn", true).await;
    f.runtime
        .outside_part_request(
            "deadline-follower",
            true,
            Some(crate::session::DEFAULT_LOCAL_USER_ID),
            &crate::local::LocalDaemonRequest::SubscribeEvents(
                crate::local::SubscribeEventsRequest {
                    targets: vec![crate::local::EventTarget {
                        kernel: f.runtime.owned.config_projection.snapshot().daemon_id,
                        session_id: f.room.clone(),
                        agent_id: Some(f.agent.clone()),
                    }],
                    sources: Vec::new(),
                },
            ),
        )
        .unwrap();
    f.begin(&f.prompt_id);
    let now = crate::session::unix_epoch_ms();
    f.runtime
        .set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: f.room.clone(),
                agent_id: f.agent.clone(),
                deadline_ms: Some(now - 1),
            },
        )
        .await
        .unwrap();
    f.runtime.tick_agent_deadlines(now).await.unwrap();
    let deadline = f
        .store()
        .agent_deadline(&f.room, &f.agent)
        .unwrap()
        .unwrap();
    assert_eq!(deadline.state, crate::local::AgentDeadlineState::Reached);
    assert!(deadline.stopped_at_ms.is_some());
    assert!(f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &f.runtime.owned.session_store.get_session(&f.room).unwrap(),
            &f.agent
        )
        .is_none());
    assert_eq!(
        f.runtime
            .owned
            .provider_store
            .get_run(&f.run)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    assert!(f
        .store()
        .agent_tasks(Some(&f.room), Some(&f.agent))
        .unwrap()
        .iter()
        .all(|task| task.state == ExecutionState::Cancelled));
    let error = f
        .runtime
        .owned
        .ensure_agent_before_deadline(&f.room, &f.agent)
        .unwrap_err();
    assert!(
        matches!(error, DaemonError::MachineRequest { code, .. } if code == "agent_deadline_reached")
    );
    // A retry after a crash or late provider output is safe and retains first receipts.
    f.runtime.tick_agent_deadlines(now + 100).await.unwrap();
    assert_eq!(
        f.store()
            .agent_deadline(&f.room, &f.agent)
            .unwrap()
            .unwrap()
            .stopped_at_ms,
        deadline.stopped_at_ms
    );
    let events = f
        .store()
        .outside_list("deadline-follower", None, 100)
        .unwrap()
        .unwrap()
        .events;
    let reached = events
        .iter()
        .filter(|event| {
            matches!(
                &event.value,
                crate::local::OutsideEventValue::Agent { reason: Some(reason), .. }
                    if reason == "deadline_reached"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(reached.len(), 1, "cleanup publishes one deadline event");
    assert_eq!(reached[0].session_id, f.room);
    assert_eq!(reached[0].agent_id.as_deref(), Some(f.agent.as_str()));
    assert!(reached[0].at_ms >= deadline.stopped_at_ms.unwrap());
}

#[tokio::test]
async fn agent_deadline_is_not_stopped_while_an_owned_source_is_still_stopping() {
    let f = fixture("agent-deadline-pending-source", false).await;
    let task = f.begin("turn-1");
    f.driver.cancel_pending.store(true, Ordering::Release);
    f.arm(&task, "wake-resisting").unwrap();
    let now = crate::session::unix_epoch_ms();
    f.runtime
        .set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: f.room.clone(),
                agent_id: f.agent.clone(),
                deadline_ms: Some(now - 1),
            },
        )
        .await
        .unwrap();
    let error = f.runtime.tick_agent_deadlines(now).await.unwrap_err();
    assert!(
        matches!(&error, DaemonError::MachineRequest { code, message }
            if code == "agent_deadline_cleanup_pending" && message.contains("wake-resisting")),
        "{error:?}"
    );
    assert_eq!(f.wake_state("wake-resisting"), "cancelling");
    let deadline = f
        .store()
        .agent_deadline(&f.room, &f.agent)
        .unwrap()
        .unwrap();
    assert_eq!(deadline.state, crate::local::AgentDeadlineState::Reached);
    assert!(
        deadline.stopped_at_ms.is_none(),
        "stopped is recorded only after the source has exited"
    );

    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence("wake-resisting", "stopped", Some(WakeEnd::Failed)))
        .unwrap();
    f.runtime
        .tick_agent_deadlines(now + 100)
        .await
        .expect("the retry settles once the source has exited");
    assert!(f
        .store()
        .agent_deadline(&f.room, &f.agent)
        .unwrap()
        .unwrap()
        .stopped_at_ms
        .is_some());
}

#[tokio::test]
async fn pending_deadline_cleanup_does_not_stop_the_sweep_for_other_wakes() {
    let f = fixture("deadline-pending-sweep", false).await;
    let task = f.begin("turn-1");
    f.driver.cancel_pending.store(true, Ordering::Release);
    f.arm(&task, "wake-resisting").unwrap();
    let now = crate::session::unix_epoch_ms();
    f.runtime
        .set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: f.room.clone(),
                agent_id: f.agent.clone(),
                deadline_ms: Some(now - 1),
            },
        )
        .await
        .unwrap();

    let other = "agent-other";
    let crate::durable_state::agent_lifecycle::Outcome::Task(other_task) = f
        .store()
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: f.room.clone(),
            agent: other.into(),
            prompt: "turn-other".into(),
            run: None,
            now,
        })
        .unwrap()
    else {
        panic!("begin returns the task");
    };
    let mut wake = f.wake(&other_task, "wake-other");
    wake.agent_id = other.into();
    f.runtime.arm_wake_source(&other_task, &wake).unwrap();
    f.driver.lost.store(true, Ordering::Release);

    f.runtime
        .sweep_agent_wakes(now)
        .await
        .expect("a pending deadline cleanup is retried, not a sweep failure");
    assert_eq!(f.wake_state("wake-resisting"), "cancelling");
    assert_eq!(
        f.wake_state("wake-other"),
        "lost",
        "the unrelated wake is still supervised"
    );
}

#[tokio::test]
async fn ev13_lost_source_raises_a_visible_alert_and_one_event() {
    let f = fixture("ev13-lost-source", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);

    f.driver.lost.store(true, Ordering::Release);
    f.runtime
        .sweep_agent_wakes(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    f.runtime
        .sweep_agent_wakes(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert_eq!(f.wake_state("wake-a"), "lost");
    assert!(
        f.notices().contains("was lost (test source vanished)"),
        "the loss is shown to the room: {}",
        f.notices()
    );
    assert_eq!(
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap().len(),
        1,
        "the waiting task hears about it exactly once"
    );
}

async fn call(
    f: &Fixture,
    token: &str,
    tool: &str,
    task: &AgentTaskExecution,
    extra: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut args = serde_json::json!({"task_id": task.task_id, "origin_prompt_id": task.prompt_id});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    f.runtime
        .dispatch_authenticated_runtime_tool_call(token, tool, args)
        .await
}

#[tokio::test]
async fn event_tools_arm_wait_and_cancel_through_the_runtime_mcp_entry() {
    let f = fixture("event-tools-mcp", true).await;
    let task = f.begin(&f.prompt_id);
    let token = {
        let app = f.app.lock().await;
        app.providers
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .expect("runtime MCP auth")
            .to_string()
    };
    let listed: Vec<String> = f
        .runtime
        .runtime_tool_specs_for_auth_token(&token)
        .into_iter()
        .map(|s| s.name)
        .filter(|n| n.starts_with("chariox.events."))
        .collect();
    assert_eq!(
        listed,
        [
            "subscribe",
            "unsubscribe",
            "subscriptions",
            "inbox",
            "ack",
            "yield",
            "timer",
            "process",
            "terminal",
            "cancel_wake",
            "wakes",
            "blocked"
        ]
        .map(|n| format!("chariox.events.{n}"))
    );

    let scheduler = {
        let runtime = f.runtime.clone();
        tokio::spawn(async move { runtime.run_agent_wake_scheduler().await })
    };
    let timer = call(
        &f,
        &token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"check in","delay_ms":1_000}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(timer["scheduler_confirmed"], true, "{timer}");
    let registration = timer["registration_id"].as_str().unwrap().to_string();
    let wake_id = timer["wake_id"].as_str().unwrap().to_string();

    let now = crate::session::unix_epoch_ms();
    for bad in [
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"deadline_ms":now - 1,"reason":"late"}),
        serde_json::json!({"registration_ids":["no-such-registration"],"inbox_cursor":0,"deadline_ms":now + 60_000,"reason":"unknown"}),
    ] {
        assert!(
            call(&f, &token, "chariox.events.yield", &task, bad)
                .await
                .is_err(),
            "an invalid wait is refused"
        );
    }
    // A12-S02: a duration passed as deadline_ms is refused with what to send instead.
    let duration = call(
        &f,
        &token,
        "chariox.events.yield",
        &task,
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"deadline_ms":3_600_000,"reason":"one hour"}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        duration.contains("absolute Unix time in milliseconds") && duration.contains("now_ms"),
        "{duration}"
    );
    let mut stale = task.clone();
    stale.prompt_id = "an-older-turn".into();
    assert!(call(
        &f,
        &token,
        "chariox.events.wakes",
        &stale,
        serde_json::json!({})
    )
    .await
    .is_err());

    let listing = call(
        &f,
        &token,
        "chariox.events.wakes",
        &task,
        serde_json::json!({}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(listing["wakes"][0]["id"], wake_id.as_str());

    call(
        &f,
        &token,
        "chariox.events.yield",
        &task,
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"deadline_ms":now + 60_000,"reason":"waiting for the clock"}),
    )
    .await
    .unwrap();
    assert!(
        call(
            &f,
            &token,
            "chariox.events.wakes",
            &task,
            serde_json::json!({})
        )
        .await
        .is_err(),
        "once the wait is accepted the turn must end"
    );

    f.store()
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: task.prompt_id.clone(),
            run: f.run.clone(),
            has_answer: true,
            cancelled: false,
            now,
        })
        .unwrap();
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
    scheduler.abort();
}

/// A10-S02: a message turn that reuses the ids of the waiting task it
/// interrupted is told that the wait continues and given its own ids, not a
/// bare refusal it escalates to the owner.
#[tokio::test]
async fn a_turn_reusing_an_earlier_waiting_tasks_ids_is_told_its_own() {
    let f = fixture("earlier-turn-origin", true).await;
    let earlier = f.begin("earlier-turn");
    f.arm(&earlier, "earlier-timer").unwrap();
    f.wait_on(&earlier, "completion-earlier-timer");
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let now = crate::session::unix_epoch_ms();
    let refusal = call(
        &f,
        &token,
        "chariox.events.yield",
        &earlier,
        serde_json::json!({"registration_ids":["completion-earlier-timer"],"inbox_cursor":0,"deadline_ms":now + 60_000,"reason":"keep waiting"}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        refusal.contains("still waiting")
            && refusal.contains(&format!("`{}`", task.task_id))
            && refusal.contains(&format!("`{}`", f.prompt_id)),
        "{refusal}"
    );
    assert_eq!(f.task(&earlier.task_id).state, ExecutionState::Waiting);
}

#[tokio::test]
async fn event_tools_cancel_a_timer_and_block_for_the_owner() {
    let f = fixture("event-tools-cancel", true).await;
    let task = f.begin(&f.prompt_id);
    let token = {
        let app = f.app.lock().await;
        app.providers
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_string()
    };
    let timer = call(
        &f,
        &token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"never needed","delay_ms":3_600_000}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = timer["wake_id"].as_str().unwrap();
    let cancelled = call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id": wake_id}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(cancelled["settled"], true, "{cancelled}");
    assert_eq!(f.wake_state(wake_id), "cancelled");
    assert!(call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id": "wake-of-nobody"}),
    )
    .await
    .is_err());

    // MP-08 / MP-09 / MP-10 / MP-11 A12-S04: a goal id is not a task id.
    let refusal = call(
        &f,
        &token,
        "chariox.events.blocked",
        &task,
        serde_json::json!({
            "task_id": format!("goal:{}", task.task_id),
            "reason": "Need the owner to pick a branch",
        }),
    )
    .await
    .expect_err("a goal id must not gain task authority");
    assert!(
        refusal
            .to_string()
            .contains(&format!("task_id is `{}`", task.task_id)),
        "the agent needs this turn's task id to correct its call: {refusal}"
    );
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Working);

    call(
        &f,
        &token,
        "chariox.events.blocked",
        &task,
        serde_json::json!({"reason": "Need the owner to pick a branch"}),
    )
    .await
    .unwrap();
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Blocked);
}

/// A03-S04: an agent that cancels a wake that already ended (here: lost) is
/// told it is settled and how it ended, so it does not arm a replacement.
#[tokio::test]
async fn cancelling_a_wake_that_already_ended_answers_settled_with_its_final_state() {
    let f = fixture("event-tools-cancel-settled", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let wake = f.arm(&task, "wake-lost").unwrap();
    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence(&wake.id, "gone", Some(WakeEnd::Lost)))
        .unwrap();
    assert_eq!(f.wake_state("wake-lost"), "lost");

    let cancelled = call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id": "wake-lost"}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(cancelled["settled"], true, "{cancelled}");
    assert_eq!(cancelled["final_state"], "lost", "{cancelled}");
    assert_eq!(f.wake_state("wake-lost"), "lost", "a lost wake stays lost");
}

/// A02-S03: an urgent message to an agent its owner stopped does not start a
/// turn; it waits, shown once to the room, until the owner prompts it again.
#[tokio::test]
async fn urgent_message_to_an_owner_stopped_agent_waits_for_the_owner() {
    let f = fixture("a02-stopped-receiver", false).await;
    let stopped = f.begin("stopped-turn");
    f.store()
        .agent_lifecycle(Operation::CancelTask {
            task: stopped.task_id.clone(),
            owner: "local".into(),
            revision: stopped.revision,
        })
        .unwrap();
    let mut message = crate::durable_state::agent_lifecycle::occurrence(
        &f.room,
        &f.agent,
        "peer-agent",
        "urgent-after-stop",
        "message",
        serde_json::json!({"message": "status now"}),
    );
    message.urgent = true;
    let Outcome::Event(message) = f
        .store()
        .agent_lifecycle(Operation::Occur(message))
        .unwrap()
    else {
        panic!("event")
    };
    let inbox_state = |f: &Fixture| {
        f.store()
            .agent_inbox(&f.room, &f.agent, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.sequence == message.sequence)
            .unwrap()
    };

    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();

    let held = inbox_state(&f);
    assert_eq!(held.state, "pending", "{held:?}");
    assert!(
        held.prompt_id.is_none(),
        "no turn started for a stopped agent: {held:?}"
    );
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.agent)
        .is_none());
    let notices = f.notices();
    assert_eq!(
        notices.matches("which you stopped").count(),
        1,
        "the waiting message is shown once: {notices}"
    );

    // The owner's next prompt lifts the stop; the message follows it.
    let next = f.begin("owner-next");
    f.store()
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: next.prompt_id.clone(),
            run: f.run.clone(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    assert!(
        inbox_state(&f).prompt_id.is_some(),
        "delivered after the owner's prompt: {:?}",
        inbox_state(&f)
    );
}

/// amsudo r1: a sweep that stalls on its sources commits a `now` older than a
/// newer sweep's. That is not a clock rollback and must not wake a live wait
/// whose deadline has not passed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_sweep_does_not_wake_a_live_wait_early() {
    let f = fixture("stalled-sweep", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    *f.driver.stall.lock().unwrap() = Some((entered_tx, release_rx));
    let runtime = f.runtime.clone();
    let stalled = tokio::spawn(async move { runtime.sweep_agent_lifecycle().await });
    tokio::task::spawn_blocking(move || {
        entered_rx.recv_timeout(std::time::Duration::from_secs(20))
    })
    .await
    .unwrap()
    .expect("the sweep probes the source before it commits");
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.store()
        .agent_lifecycle(Operation::Sweep {
            now: crate::session::unix_epoch_ms(),
            vault_wait_recipients: Vec::new(),
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        })
        .unwrap();
    release_tx.send(()).unwrap();
    stalled.await.unwrap().unwrap();
    let early = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "deadline_reached")
        .count();
    assert_eq!(early, 0, "no deadline has passed");
}

async fn runtime_token(f: &Fixture) -> String {
    let app = f.app.lock().await;
    app.providers
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .expect("runtime MCP auth")
        .to_string()
}

fn open_terminal(f: &Fixture, command: &str) -> String {
    f.runtime
        .owned
        .room_terminals
        .open(crate::room_terminal::OpenRoomTerminal {
            project_environment: Default::default(),
            session_id: f.room.clone(),
            working_directory: f.runtime.room_project_directory(&f.room).unwrap(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: crate::room_terminal::TerminalActor::Agent(f.agent.clone()),
            command: Some(vec!["sh".into(), "-c".into(), command.into()]),
        })
        .unwrap()
        .terminal_id
}

/// TERM-AGENT-WAIT notify half: a command that finishes while the agent
/// waits on the event reaches it once, with the exit code and the screen.
/// Delivering the wake turn is the ledger's and is covered by the driver tests above.
#[tokio::test]
async fn terminal_finish_reports_once_to_the_waiting_agent_with_exit_code_and_screen() {
    let f = fixture("terminal-wake-finish", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let terminal = open_terminal(&f, "sleep 1; echo SCREEN-MARK; exit 4");
    let armed = call(
        &f,
        &token,
        "chariox.events.terminal",
        &task,
        serde_json::json!({"label":"build","terminal_id":terminal,"until":["finished"]}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = armed["wake_id"].as_str().unwrap().to_string();
    assert_eq!(f.wake_state(&wake_id), "running");
    f.wait_on(&task, armed["registration_id"].as_str().unwrap());

    let events = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if !events.is_empty() {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the finished terminal reports to the waiting agent");
    assert_eq!(events.len(), 1, "reported once");
    let answer = &events[0].payload["public_answer"];
    assert_eq!(answer["exit_code"], 4, "{answer}");
    assert!(
        answer["screen_text"]
            .as_str()
            .unwrap()
            .contains("SCREEN-MARK"),
        "{answer}"
    );
    assert_eq!(f.wake_state(&wake_id), "exited");
}

/// A fired watch is already removed from the terminal, so a rejected ledger
/// write must not discard the occurrence: it is retried until it commits.
#[tokio::test]
async fn a_rejected_terminal_occurrence_is_retried_until_the_ledger_commits() {
    let f = fixture("terminal-wake-retry", true).await;
    let received = f._worktree.path().join("provider-input");
    {
        let mut app = f.app.lock().await;
        app.pty_mut().remove_process(&f.run).unwrap();
        app.pty_mut()
            .spawn(crate::pty::PtySpawnRequest {
                agent: None,
                process_key: f.run.clone(),
                provider_run_id: f.run.clone(),
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "stty raw -echo; exec cat > \"$1\"".into(),
                    "wake-provider".into(),
                    received.display().to_string(),
                ],
                env: Default::default(),
                env_remove: Vec::new(),
                working_directory: None,
                cols: 80,
                rows: 24,
            })
            .unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !received.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the provider transport is ready to receive the wake turn");
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    f.runtime
        .terminal_wake_driver
        .as_ref()
        .expect("the runtime registers the terminal wake driver")
        .sink()
        .reject_next_occurrences(2);
    let terminal = open_terminal(&f, "sleep 1; echo RETRY-MARK; exit 0");
    let armed = call(
        &f,
        &token,
        "chariox.events.terminal",
        &task,
        serde_json::json!({"label":"build","terminal_id":terminal,"until":["finished"]}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = armed["wake_id"].as_str().unwrap().to_string();
    f.wait_on(&task, armed["registration_id"].as_str().unwrap());
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    f.runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &f.agent)
        .expect("the yielding provider turn ends before its terminal finishes");

    let events = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if !events.is_empty() {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the rejected occurrence is delivered once the writer accepts it again");
    assert_eq!(events.len(), 1, "exactly one occurrence");
    assert_eq!(f.wake_state(&wake_id), "exited");
    // No lifecycle sweep runs here: the retry itself must schedule delivery,
    // and the provider process must read the wake turn from its transport.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let input = std::fs::read_to_string(&received).unwrap();
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if input.contains("Inbox: at least 1 new event") && events[0].state == "accepted" {
                assert_eq!(events.len(), 1, "exactly one delivered occurrence");
                let woken = f.task(&task.task_id);
                assert_eq!(woken.state, ExecutionState::Working);
                assert_eq!(woken.prompt_id, events[0].prompt_id.clone().unwrap());
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the retry delivers the wake turn to the provider without a sweep");
}

#[tokio::test]
async fn terminal_wake_refuses_a_terminal_that_is_not_running_here() {
    let f = fixture("terminal-wake-unknown", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let error = call(
        &f,
        &token,
        "chariox.events.terminal",
        &task,
        serde_json::json!({"label":"x","terminal_id":"term-nope","until":["finished"]}),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("no running terminal"), "{error}");
    assert!(f
        .store()
        .agent_wakes(Some(&f.room), Some(&f.agent))
        .unwrap()
        .iter()
        .all(|w| w.kind != "terminal" || w.state != "running"));
}

/// The terminal wake driver holds the runtime and the runtime owns the Room terminal service
/// and the wake registry the driver is registered in. Dropping the runtime must free all of
/// it, or a kernel rebuilt in the same process can never take over the durable state lock.
#[tokio::test]
async fn dropping_the_runtime_frees_the_terminal_wake_driver() {
    let _worktree = crate::test_support::TestWorktree::new("terminal-driver-drop");
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap(),
    ));
    let runtime = owned_runtime_state(&app).await;
    let driver = Arc::downgrade(
        runtime
            .terminal_wake_driver
            .as_ref()
            .expect("the runtime registers the terminal wake driver"),
    );
    let kernel = Arc::downgrade(&app);
    drop(runtime);
    drop(app);
    assert!(
        driver.upgrade().is_none(),
        "the driver outlived its runtime"
    );
    assert!(
        kernel.upgrade().is_none(),
        "the kernel outlived its runtime"
    );
}

#[path = "async_popup.rs"]
mod async_popup;

async fn run_token(f: &Fixture) -> String {
    f.app
        .lock()
        .await
        .providers
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned()
}

/// A browser that failed to start is not a permission question: the "allow browser access" card
/// is refused until a start succeeds again, with what to do instead.
#[tokio::test]
async fn blocked_for_browser_access_is_refused_after_a_failed_browser_start() {
    let f = fixture("owner-browser-card-start-failed", true).await;
    let task = f.begin(&f.prompt_id);
    let token = run_token(&f).await;
    let host = &f.runtime.owned.kernel_browser_host;
    host.set_focus("local", Some(&f.agent));
    host.load("local", &f.agent).unwrap();
    let root = std::env::temp_dir().join(format!(
        "chariox-start-failed-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let broken = root.join("broken.sh");
    std::fs::write(&broken, "exit 1\n").unwrap();
    host.install_fixture_backend("local", &broken, &root);
    let admission = host.admit("local", &f.agent).unwrap();
    host.protected_request_admitted(
        "local",
        Some(&admission),
        "host.browser",
        serde_json::json!({"op":"start"}),
        serde_json::Value::Null,
    )
    .expect_err("the controller never comes up");
    assert!(host.start_failed("local"));
    let args = serde_json::json!({
        "reason":"The browser did not start.", "owner_action":"browser_access"
    });
    let error = call(&f, &token, "chariox.events.blocked", &task, args.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("did not start") && error.contains("start once more"),
        "{error}"
    );
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(
        session.active_interactions().is_empty(),
        "no card for a browser that failed to start"
    );
    assert!(!matches!(
        f.task(&task.task_id).state,
        ExecutionState::Blocked
    ));
    // The same words without owner_action stay an ordinary block.
    call(
        &f,
        &token,
        "chariox.events.blocked",
        &task,
        serde_json::json!({"reason":"The browser did not start."}),
    )
    .await
    .unwrap();
    // A start that works clears the failure: the card is a question about access again.
    f.runtime
        .install_unstarted_kernel_browser_fixture("local", &root);
    host.protected_request_admitted(
        "local",
        Some(&host.admit("local", &f.agent).unwrap()),
        "host.browser",
        serde_json::json!({"op":"start"}),
        serde_json::Value::Null,
    )
    .unwrap();
    assert!(!host.start_failed("local"));
    std::fs::remove_dir_all(root).unwrap();
}

/// The controller stays alive while Chrome fails to start; the explicit `start` the refusal asks
/// for is the retry, and it must clear the failure on that same controller.
#[tokio::test]
async fn an_explicit_start_on_the_live_controller_clears_a_failed_start() {
    let f = fixture("owner-browser-card-live-retry", true).await;
    let task = f.begin(&f.prompt_id);
    let token = run_token(&f).await;
    let host = &f.runtime.owned.kernel_browser_host;
    host.set_focus("local", Some(&f.agent));
    host.load("local", &f.agent).unwrap();
    let root = std::env::temp_dir().join(format!(
        "chariox-start-live-retry-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    f.runtime
        .install_unstarted_kernel_browser_fixture("local", &root);
    let script = root.join("MD5-controller.sh");
    let source = std::fs::read_to_string(&script).unwrap();
    let healthy = r#"*'"op":"start"'*) printf 'start\n' >> "$1/starts"; "#;
    assert!(source.contains(healthy));
    // The controller answers every request; only Chrome's start fails while the flag file exists.
    let failing = r#"*'"op":"start"'*) if test -e "$1/chrome-down"; then printf '{"id":%s,"ok":false,"error":{"code":"browser_start_failed","message":"chrome did not start"}}\n' "$id"; continue; fi; printf 'start\n' >> "$1/starts"; "#;
    std::fs::write(&script, source.replace(healthy, failing)).unwrap();
    std::fs::write(root.join("chrome-down"), "").unwrap();
    // The first agent read launches the controller; its implicit start then fails.
    let read = || {
        host.protected_request_admitted(
            "local",
            Some(&host.admit("local", &f.agent).unwrap()),
            "host.browser",
            serde_json::json!({"op":"state"}),
            serde_json::Value::Null,
        )
    };
    read().expect_err("Chrome does not start");
    assert!(host.start_failed("local"));
    let args = serde_json::json!({
        "reason":"I need the signed-in tab.", "owner_action":"browser_access"
    });
    let refusal = call(&f, &token, "chariox.events.blocked", &task, args.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("start once more"), "{refusal}");
    // An explicit start that fails again keeps the failure on the live controller.
    let start = || {
        host.protected_request_admitted(
            "local",
            Some(&host.admit("local", &f.agent).unwrap()),
            "host.browser",
            serde_json::json!({"op":"start"}),
            serde_json::Value::Null,
        )
    };
    start().expect_err("Chrome is still down");
    assert!(host.start_failed("local"));
    // The retry the refusal asked for succeeds on the same controller.
    std::fs::remove_file(root.join("chrome-down")).unwrap();
    start().unwrap();
    assert!(
        !host.start_failed("local"),
        "a successful start clears the failure"
    );
    call(&f, &token, "chariox.events.blocked", &task, args)
        .await
        .expect("the owner can approve the signed-in tab again");
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(session
        .active_interactions()
        .last()
        .unwrap()
        .choices()
        .iter()
        .any(|choice| choice.id() == "allow_browser"));
    std::fs::remove_dir_all(root).unwrap();
}

/// A loaded grant that does not hold the tab the task needs is a missing resource, not a browser
/// that failed to start: the owner can still approve it.
#[tokio::test]
async fn blocked_for_browser_access_is_offered_when_a_loaded_grant_lacks_the_tab() {
    let f = fixture("owner-browser-card-partial", true).await;
    let task = f.begin(&f.prompt_id);
    let token = run_token(&f).await;
    let host = &f.runtime.owned.kernel_browser_host;
    host.request_grant(
        "local",
        &f.agent,
        &f.prompt_id,
        Duration::from_secs(600),
        None,
        &f.room,
    )
    .unwrap();
    let admission = host.admit("local", &f.agent).unwrap();
    let tab = crate::local::UserDomainResource::BrowserTab {
        tab_id: "existing-signed-in-tab".into(),
    };
    let refusal = host
        .claim_resource(Some(&admission), tab, false)
        .unwrap_err();
    assert!(refusal.contains("not_focused_agent"), "{refusal}");
    call(
        &f,
        &token,
        "chariox.events.blocked",
        &task,
        serde_json::json!({
            "reason":"I need the signed-in tab.", "owner_action":"browser_access"
        }),
    )
    .await
    .expect("the owner can approve the missing tab");
    assert!(matches!(
        f.task(&task.task_id).state,
        ExecutionState::Blocked
    ));
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(session
        .active_interactions()
        .last()
        .unwrap()
        .choices()
        .iter()
        .any(|choice| choice.id() == "allow_browser"));
}

#[tokio::test]
async fn owner_browser_decision_restores_access_and_rejects_stale_answers() {
    for stale in [false, true] {
        let f = fixture("owner-browser-decision", true).await;
        let task = f.begin(&f.prompt_id);
        let token = f
            .app
            .lock()
            .await
            .providers
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        call(&f, &token, "chariox.events.blocked", &task, serde_json::json!({
            "reason":"Browser access ended. Please allow browser access.", "owner_action":"browser_access"
        })).await.unwrap();
        let blocked = f.task(&task.task_id);
        f.runtime
            .ensure_task_owner_interaction(blocked.clone())
            .await
            .unwrap();
        let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
        let card = session.active_interactions().last().unwrap();
        assert!(
            card.choices()
                .iter()
                .any(|choice| choice.id() == "allow_browser"),
            "a browser refusal offers an approval that restores access in place"
        );
        assert!(!card.message().contains("chariox.") && !card.message().contains("user_domain"));
        assert_eq!(card.timeout_sec(), None);
        assert!(!f
            .runtime
            .owned
            .kernel_browser_host
            .has_grant("local", &f.agent));
        if stale {
            f.store()
                .agent_lifecycle(Operation::OwnerResponse {
                    task: task.task_id.clone(),
                    revision: blocked.blocked_revision,
                    resume: false,
                    now: crate::session::unix_epoch_ms(),
                })
                .unwrap();
        }
        f.runtime
            .owned
            .resolve_attributed_terminal_runtime_interaction(
                &f.room,
                card.id(),
                "allow_browser",
                None,
                Some("local"),
                true,
                Some(crate::local::KernelConnectionClass::Terminal),
                None,
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            f.runtime
                .owned
                .kernel_browser_host
                .has_grant("local", &f.agent),
            !stale
        );
    }
}

#[tokio::test]
async fn owner_browser_reload_after_a_wake_offers_approval_in_place() {
    let f = fixture("browser-owner-after-wake", true).await;
    let task = f.begin(&f.prompt_id);
    let token = {
        let mut app = f.app.lock().await;
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &f.room,
                "task-continuation",
                crate::attachment::ClientCapabilityLevel::AutomationOnly,
            ))
            .unwrap();
        let session = app.sessions().get_session(&f.room).unwrap();
        let prompts = app.prompt_state_owner();
        let active = prompts
            .active_prompt_for_agent_snapshot(&session, &f.agent)
            .unwrap();
        let resumed = active.clone().with_source_attachment(&attachment);
        assert!(prompts.replace_active_prompt_if_matches(&session, &f.agent, &active, resumed));
        app.providers
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned()
    };
    let result = f
        .runtime
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.load_kernel_browser",
            serde_json::json!({}),
        )
        .await;
    assert!(
        result.is_ok(),
        "a retained task asks for browser approval instead of failing again: {result:?}"
    );
    assert_eq!(
        f.task(&task.task_id).owner_action,
        Some(crate::durable_state::agent_lifecycle::TaskOwnerAction::BrowserAccess)
    );
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Blocked);
    assert!(!f
        .runtime
        .owned
        .kernel_browser_host
        .has_grant("local", &f.agent));
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(session.active_interactions().iter().any(|card| card
        .choices()
        .iter()
        .any(|choice| choice.id() == "allow_browser")));
}

impl Fixture {
    /// The kernel-owned rows of the agent's waits: (history sequence, merge key, row JSON).
    fn wait_rows(&self) -> Vec<(u64, String, serde_json::Value)> {
        self.runtime
            .owned
            .operational_history_store
            .load_session_events(&self.room, Some(&self.agent))
            .unwrap()
            .into_iter()
            .filter_map(|event| {
                let entry = event.to_session_history_entry()?;
                let key = entry.merge_key.clone()?;
                (entry.kind == crate::history::SessionHistoryEntryKind::ProviderTool
                    && key.starts_with("wait:"))
                .then(|| {
                    (
                        event.sequence,
                        key,
                        serde_json::from_str(&entry.text).unwrap(),
                    )
                })
            })
            .collect()
    }
}

// Landing 9 sub-panes: a wait is a row of the agent's transcript. The kernel writes it when the
// wait starts and updates that same row when the agent wakes, so no client has to invent it.
#[tokio::test]
async fn a_yield_writes_one_wait_row_and_the_wake_updates_it_in_place() {
    let f = fixture("wait-row-in-place", true).await;
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let prompt = f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.agent)
        .unwrap();
    let task = f.begin(&f.prompt_id);
    let wake = f.arm(&task, "wake-a").unwrap();
    let now = crate::session::unix_epoch_ms();
    f.store()
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            registrations: vec![wake.registration_id.clone()],
            cursor: 0,
            deadline: Some(now + 60_000),
            reason: "Wait for the test source".into(),
            now,
        })
        .unwrap();
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.room, &f.agent, Some(&f.run))
        .unwrap();
    let settlement = f
        .runtime
        .owned
        .settle_agent_task(&f.room, &f.agent, &prompt, &f.run, false)
        .unwrap();
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();

    let waiting = f.wait_rows();
    assert_eq!(waiting.len(), 1, "one wait, one row: {waiting:?}");
    let (sequence, key, row) = &waiting[0];
    assert_eq!(row["status"], "running", "{row}");
    assert_eq!(row["tool"], "chariox.wait", "{row}");
    assert_eq!(row["input"]["reason"], "Wait for the test source", "{row}");
    assert_eq!(row["input"]["deadline_ms"], now + 60_000, "{row}");
    assert!(row["finished_at_ms"].is_null(), "{row}");

    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while f.task(&task.task_id).state != ExecutionState::Working {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the wake reaches the waiting task");

    let woken = f.wait_rows();
    assert_eq!(woken.len(), 1, "the wake updates the row: {woken:?}");
    let (woken_sequence, woken_key, row) = &woken[0];
    assert_eq!((woken_sequence, woken_key), (sequence, key));
    assert_eq!(row["status"], "completed", "{row}");
    assert_eq!(row["woken_by"]["kind"], "output_matched", "{row}");
    let started = row["started_at_ms"].as_u64().unwrap();
    assert!(row["finished_at_ms"].as_u64().unwrap() >= started, "{row}");
}

// A wait the owner stops does not stay a running row in the transcript: the same row says it was cancelled.
// A yield without an explicit deadline reports the kernel's default deadline in its wait row.
#[tokio::test]
async fn a_yield_without_a_deadline_writes_a_wait_row_with_the_default_deadline() {
    let f = fixture("wait-row-no-deadline", true).await;
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let prompt = f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.agent)
        .unwrap();
    let task = f.begin(&f.prompt_id);
    let wake = f.arm(&task, "wake-a").unwrap();
    f.store()
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            registrations: vec![wake.registration_id.clone()],
            cursor: 0,
            deadline: None,
            reason: "Wait with no deadline".into(),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.room, &f.agent, Some(&f.run))
        .unwrap();
    let settlement = f
        .runtime
        .owned
        .settle_agent_task(&f.room, &f.agent, &prompt, &f.run, false)
        .unwrap();
    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();

    let waiting = f.wait_rows();
    assert_eq!(waiting.len(), 1, "one wait, one row: {waiting:?}");
    let row = &waiting[0].2;
    assert_eq!(row["status"], "running", "{row}");
    assert!(
        row["input"]["deadline_ms"]
            .as_u64()
            .is_some_and(|deadline| deadline
                >= row["started_at_ms"].as_u64().unwrap()
                    + crate::durable_state::agent_lifecycle::DEFAULT_WAIT_TIMEOUT_MS),
        "the row names the default deadline: {row}"
    );
}

#[tokio::test]
async fn stopping_a_waiting_task_closes_its_wait_row_in_place() {
    let f = fixture("wait-row-cancelled", true).await;
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let prompt = f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.agent)
        .unwrap();
    let task = f.begin(&f.prompt_id);
    let wake = f.arm(&task, "wake-a").unwrap();
    let now = crate::session::unix_epoch_ms();
    f.store()
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            registrations: vec![wake.registration_id.clone()],
            cursor: 0,
            deadline: Some(now + 60_000),
            reason: "Wait for the test source".into(),
            now,
        })
        .unwrap();
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.room, &f.agent, Some(&f.run))
        .unwrap();
    let settlement = f
        .runtime
        .owned
        .settle_agent_task(&f.room, &f.agent, &prompt, &f.run, false)
        .unwrap();
    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();
    let before = f.wait_rows();
    assert_eq!(before.len(), 1);

    let waiting = f.task(&task.task_id);
    let Outcome::Task(cancelled) = f
        .store()
        .agent_lifecycle(Operation::CancelTask {
            task: task.task_id.clone(),
            owner: "local".into(),
            revision: waiting.revision,
        })
        .unwrap()
    else {
        panic!("the cancelled task")
    };
    f.runtime
        .cancel_agent_task_resources(&cancelled)
        .await
        .unwrap();

    let after = f.wait_rows();
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!((after[0].0, &after[0].1), (before[0].0, &before[0].1));
    assert_eq!(after[0].2["status"], "completed", "{}", after[0].2);
    assert_eq!(
        after[0].2["woken_by"]["kind"], "cancelled",
        "{}",
        after[0].2
    );
}

// Live drill, 11-11: a sub-agent's end wakes the Room agent through an event that names no task, and the woken turn runs as a
// task of its own, so the yielded task still reads waiting. The wait row ends with the wake all the same, because the
// event is the end of the source the wait is on.
#[tokio::test]
async fn an_event_that_names_no_task_still_ends_the_wait_row_of_the_agent_it_wakes() {
    let f = fixture("wait-row-unnamed-wake", true).await;
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let prompt = f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.agent)
        .unwrap();
    let task = f.begin(&f.prompt_id);
    let wake = f.arm(&task, "wake-a").unwrap();
    let now = crate::session::unix_epoch_ms();
    f.store()
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            registrations: vec![wake.registration_id.clone()],
            cursor: 0,
            deadline: Some(now + 60_000),
            reason: "Wait for the test source".into(),
            now,
        })
        .unwrap();
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.room, &f.agent, Some(&f.run))
        .unwrap();
    let settlement = f
        .runtime
        .owned
        .settle_agent_task(&f.room, &f.agent, &prompt, &f.run, false)
        .unwrap();
    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();
    assert_eq!(f.wait_rows()[0].2["status"], "running");

    let Outcome::Event(event) = f
        .store()
        .agent_lifecycle(Operation::Occur(
            crate::durable_state::agent_lifecycle::occurrence(
                &f.room,
                &f.agent,
                "wake-a",
                "child-ended",
                "child_finished",
                serde_json::json!({"child": "a-child"}),
            ),
        ))
        .unwrap()
    else {
        panic!("event")
    };
    f.runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let delivered = f
                .store()
                .agent_inbox(&f.room, &f.agent, 0)
                .unwrap()
                .into_iter()
                .any(|e| e.sequence == event.sequence && e.state == "accepted");
            if delivered {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the event is delivered as a turn");

    let rows = f.wait_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].2["status"], "completed", "{}", rows[0].2);
    assert_eq!(
        rows[0].2["woken_by"]["kind"], "child_finished",
        "{}",
        rows[0].2
    );
}

impl Fixture {
    /// A task that yielded on `wake-a` and settled to Waiting. Its wait row is not written yet:
    /// `finish_agent_task_settlement` does that, as the prompt settlement does after the commit.
    async fn settle_a_yield(
        &self,
    ) -> (
        AgentTaskExecution,
        Option<(AgentTaskExecution, bool)>,
        crate::session::PromptQueueItem,
    ) {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.room)
            .unwrap();
        let prompt = self
            .runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &self.agent)
            .unwrap();
        let task = self.begin(&self.prompt_id);
        let wake = self.arm(&task, "wake-a").unwrap();
        let now = crate::session::unix_epoch_ms();
        self.store()
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                registrations: vec![wake.registration_id.clone()],
                cursor: 0,
                deadline: Some(now + 60_000),
                reason: "Wait for the test source".into(),
                now,
            })
            .unwrap();
        self.runtime
            .owned
            .complete_local_prompt_without_advance(&self.room, &self.agent, Some(&self.run))
            .unwrap();
        let settlement = self
            .runtime
            .owned
            .settle_agent_task(&self.room, &self.agent, &prompt, &self.run, false)
            .unwrap();
        assert_eq!(self.task(&task.task_id).state, ExecutionState::Waiting);
        (task, settlement, prompt)
    }

    fn occur(&self, source: &str, id: &str, kind: &str, payload: serde_json::Value) -> u64 {
        let Outcome::Event(event) = self
            .store()
            .agent_lifecycle(Operation::Occur(
                crate::durable_state::agent_lifecycle::occurrence(
                    &self.room,
                    &self.agent,
                    source,
                    id,
                    kind,
                    payload,
                ),
            ))
            .unwrap()
        else {
            panic!("event")
        };
        event.sequence
    }

    async fn until_inbox_event(&self, sequence: u64, state: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let reached = self
                    .store()
                    .agent_inbox(&self.room, &self.agent, 0)
                    .unwrap()
                    .into_iter()
                    .any(|e| e.sequence == sequence && e.state == state);
                if reached {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the event reaches {state}"));
    }
}

// A message wakes the agent for a turn but leaves the task waiting on its source: the wait goes on,
// so its row stays running until the source ends it.
#[tokio::test]
async fn a_message_does_not_end_the_wait_row_of_a_task_that_keeps_waiting() {
    let f = fixture("wait-row-message", true).await;
    let (task, settlement, prompt) = f.settle_a_yield().await;
    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();
    assert_eq!(f.wait_rows()[0].2["status"], "running");

    let sequence = f.occur(
        "a-peer",
        "peer-says-hi",
        "message",
        serde_json::json!({"text": "hi"}),
    );
    f.runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    f.until_inbox_event(sequence, "accepted").await;

    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
    let rows = f.wait_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].2["status"], "running", "{}", rows[0].2);
}

// An attempt the provider refuses runs nothing: the task is back to waiting and so is its row.
#[tokio::test]
async fn a_refused_wake_leaves_the_wait_row_running() {
    let f = fixture("wait-row-refused", true).await;
    let (task, settlement, prompt) = f.settle_a_yield().await;
    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();
    let sequence = f.occur(
        "wake-a",
        "line-1",
        "output_matched",
        serde_json::json!({"task_id": task.task_id}),
    );

    // What the delivery does before it dispatches the wake.
    let waiting = f.runtime.owned.waiting_tasks(&f.room, &f.agent);
    let Outcome::Event(attempt) = f
        .store()
        .agent_lifecycle(Operation::Attempt {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequence,
            prompt: "prompt-wake".into(),
            target: None,
            run: None,
            now: crate::session::unix_epoch_ms(),
            work: None,
            through: sequence,
        })
        .unwrap()
    else {
        panic!("attempt")
    };
    f.runtime.owned.record_woken_waits(&waiting, &attempt);
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Working);

    f.runtime
        .record_agent_delivery_receipt(&f.room, &f.agent, sequence, "rejected")
        .unwrap();

    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
    let rows = f.wait_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].2["status"], "running", "{}", rows[0].2);
    assert!(rows[0].2["woken_by"].is_null(), "{}", rows[0].2);
}

// The source can end right after the yield: a wake that lands between the commit of Waiting and the
// row of that wait moves the task on, and the row written afterwards must not stay running.
#[tokio::test]
async fn a_wake_between_settlement_and_its_finish_leaves_no_running_wait_row() {
    let f = fixture("wait-row-raced", true).await;
    let (task, settlement, prompt) = f.settle_a_yield().await;
    let sequence = f.occur(
        "wake-a",
        "line-1",
        "output_matched",
        serde_json::json!({"task_id": task.task_id}),
    );
    f.runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    f.until_inbox_event(sequence, "accepted").await;
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Working);

    f.runtime
        .finish_agent_task_settlement(settlement, &prompt)
        .await
        .unwrap();

    let running: Vec<_> = f
        .wait_rows()
        .into_iter()
        .filter(|(_, _, row)| row["status"] == "running")
        .collect();
    assert!(running.is_empty(), "a wait that already ended: {running:?}");
}
