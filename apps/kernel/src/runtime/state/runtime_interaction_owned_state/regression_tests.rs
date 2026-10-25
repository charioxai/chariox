use super::*;
use crate::{
    config::DaemonConfig,
    session::{
        RuntimeInteraction, RuntimeInteractionChoice, RuntimeSession, DEFAULT_LOCAL_USER_ID,
    },
};
use tokio::sync::oneshot;

struct Fixture {
    state: KernelRuntimeState,
    session: String,
}
impl Fixture {
    fn new() -> Self {
        Self::with_session_id(format!("decision-cleanup-{:016x}", rand::random::<u64>()))
    }
    fn with_session_id(session_id: String) -> Self {
        Self::with_session_config(session_id, false)
    }
    fn with_room_tools() -> Self {
        Self::with_session_config(format!("am2-sweep-{:016x}", rand::random::<u64>()), true)
    }
    fn with_session_config(session_id: String, room_tools: bool) -> Self {
        let mut config = DaemonConfig::for_tests();
        config.room_agent_tools = room_tools;
        let app = DaemonApp::bootstrap(config).unwrap();
        let session = RuntimeSession::new(
            session_id,
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        let session_id = session.id().to_owned();
        app.sessions_mut().restore_session(session);
        let config = app.config_projection_store();
        let sessions = app.session_state_store();
        let agents = app.agents().clone();
        let attachments = app.attachments().clone();
        let providers = app.providers().clone();
        let tracking = app.provider_process_tracking_store();
        let slices = app.slices();
        let projection = app.session_state_projection_store();
        let runs = app.provider_run_projection_store();
        let history = app.operational_history_store();
        let durable = app.durable_state_store();
        let prompts = app.prompt_state_owner();
        let turns = app.active_turn_store();
        let activity = app.prompt_activity_store();
        let claims = app.prompt_workspace_claim_store();
        let outputs = app.structured_output_record_store();
        let terminal = app.terminal_stream_store();
        let workflow = app.workflow_design_event_store();
        let meta = app.metaagent_event_store();
        let workspace = app.workspace_coordinator();
        let state = KernelRuntimeState::new_with_owned_state(
            Arc::new(Mutex::new(app)),
            config,
            sessions,
            agents,
            attachments,
            providers,
            tracking,
            slices,
            projection,
            runs,
            history,
            durable,
            prompts,
            turns,
            activity,
            claims,
            outputs,
            terminal,
            workflow,
            meta,
            workspace,
        );
        Self {
            state,
            session: session_id,
        }
    }
    fn id(&self, name: &str) -> String {
        format!("{}:{name}", self.session)
    }
    fn register(
        &self,
        name: &str,
    ) -> Result<oneshot::Receiver<super::super::PendingInteractionResolution>, DaemonError> {
        let id = self.id(name);
        let (send, receive) = oneshot::channel();
        let interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Install App?",
            "Review this installation",
            vec![
                RuntimeInteractionChoice::new("deny", "Cancel", "deny", None),
                RuntimeInteractionChoice::new("allow", "Install", "allow", None),
            ],
        );
        self.state.owned.register_runtime_interaction(
            &self.session,
            interaction,
            send,
            Some(DEFAULT_LOCAL_USER_ID),
            None,
            None,
        )?;
        Ok(receive)
    }
    fn active_ids(&self) -> Vec<String> {
        self.state
            .owned
            .session_store
            .get_session(&self.session)
            .unwrap()
            .active_interactions()
            .iter()
            .map(|interaction| interaction.id().to_owned())
            .collect()
    }
}

#[tokio::test]
async fn foreign_kernel_quotas_and_lexically_earlier_entries_do_not_starve_local_cleanup() {
    let suffix = rand::random::<u64>();
    let foreign = (0..4)
        .map(|index| Fixture::with_session_id(format!("00-foreign-{index}-{suffix:x}")))
        .collect::<Vec<_>>();
    let mut foreign_receivers = Vec::new();
    for fixture in &foreign {
        for index in 0..8 {
            foreign_receivers.push(fixture.register(&format!("decision-{index}")).unwrap());
        }
        assert!(fixture.register("over-owner-limit").is_err());
    }
    let local = Fixture::with_session_id(format!("zz-local-{suffix:x}"));
    let mut local_receiver = local.register("local").unwrap();
    // All 32 foreign candidates sort before the local one in the shared map.
    // A shutdown sweep must select this kernel before applying the batch limit.
    local.state.owned.sweep_kernel_operation_interactions(true);
    assert_eq!(local_receiver.try_recv().unwrap().status, "timed_out");
    assert!(local.active_ids().is_empty());
    for receiver in &mut foreign_receivers {
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }
    for fixture in &foreign {
        assert_eq!(fixture.active_ids().len(), 8);
    }
}

#[tokio::test]
async fn shared_session_ids_cannot_cross_kernel_resolution_or_cleanup_but_cloned_handles_can() {
    let id = format!("shared-session-{:016x}", rand::random::<u64>());
    let owner = Fixture::with_session_id(id.clone());
    let foreign = Fixture::with_session_id(id);
    let mut receiver = owner.register("same-id").unwrap();
    // A restored foreign kernel may contain the same projected session and
    // interaction IDs. The pending responder still belongs only to its owner.
    let snapshot = owner
        .state
        .owned
        .session_store
        .get_session(&owner.session)
        .unwrap();
    foreign
        .state
        .owned
        .session_store
        .write()
        .restore_session(snapshot);
    assert!(foreign
        .state
        .owned
        .resolve_runtime_interaction(
            &foreign.session,
            &owner.id("same-id"),
            "allow",
            None,
            Some(DEFAULT_LOCAL_USER_ID),
            false,
        )
        .is_err());
    foreign
        .state
        .owned
        .timeout_runtime_interaction(&foreign.session, &owner.id("same-id"))
        .unwrap();
    foreign
        .state
        .owned
        .sweep_kernel_operation_interactions(true);
    assert_eq!(owner.active_ids(), vec![owner.id("same-id")]);
    assert!(matches!(
        receiver.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    let same_kernel = owner.state.clone();
    same_kernel
        .owned
        .resolve_runtime_interaction(
            &owner.session,
            &owner.id("same-id"),
            "allow",
            None,
            Some(DEFAULT_LOCAL_USER_ID),
            false,
        )
        .unwrap();
    assert_eq!(
        receiver.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
    assert!(owner.active_ids().is_empty());
}

#[tokio::test]
async fn pruning_dead_store_tokens_closes_only_kernel_operation_responders() {
    let sessions = crate::session::SessionStateStore::new(crate::session::SessionService::new(
        &DaemonConfig::for_tests(),
    ));
    let cloned_sessions = sessions.clone();
    let pending = super::super::PendingInteractionStore::default();
    let (kernel_sender, mut kernel_receiver) = oneshot::channel();
    let (agent_sender, mut agent_receiver) = oneshot::channel();
    for (id, sender, owner) in [
        (
            "kernel",
            kernel_sender,
            Some(DEFAULT_LOCAL_USER_ID.to_owned()),
        ),
        ("agent", agent_sender, None),
    ] {
        pending.write().insert(
            id.into(),
            super::super::PendingInteraction {
                goal_approval: None,
                kernel_wide_interaction: None,
                agent_lifetime: None,
                session_id: "same-session".into(),
                session_store_identity: sessions.weak_identity(),
                user_domain_interaction: None,
                kernel_operation_owner: owner,
                terminal_credential_owner: None,
                kernel_operation_deadline: None,
                passkey_prompt: None,
                responder: Arc::new(std::sync::Mutex::new(Some(sender))),
            },
        );
    }
    let _mutation = pending.mutation.lock().unwrap();
    drop(sessions);
    pending.prune_abandoned_kernel_owners();
    assert_eq!(
        pending.write().len(),
        2,
        "another router retains the owning store"
    );
    drop(cloned_sessions);
    pending.prune_abandoned_kernel_owners();
    assert!(!pending.write().contains_key("kernel"));
    assert!(pending.write().contains_key("agent"));
    assert!(matches!(
        kernel_receiver.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));
    assert!(matches!(
        agent_receiver.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.state.owned.sweep_kernel_operation_interactions(true);
    }
}

#[tokio::test]
async fn a02_unloaded_room_cannot_abort_live_room_supervision() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    let f = Fixture::with_room_tools();
    let store = &f.state.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: "unloaded-room".into(),
            agent: "old-agent".into(),
            prompt: "a-old-blocked".into(),
            run: None,
            now: 1,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Block {
            task: "a-old-blocked".into(),
            prompt: "a-old-blocked".into(),
            reason: "Original receipt requires owner reconciliation".into(),
        })
        .unwrap();
    let before = store.agent_tasks(Some("unloaded-room"), None).unwrap();
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "idle-agent".into(),
            prompt: "z-current-idle".into(),
            run: None,
            now: 1,
        })
        .unwrap();
    // A plain turn with no live executor is cancelled; ledger work (an obligation)
    // is what makes the sweep block it for the owner.
    store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "idle-agent".into(),
            prompt: "z-current-idle".into(),
            run: None,
            id: "z-current-obligation".into(),
            kind: "delegate".into(),
            resource: Some("child".into()),
            now: 1,
        })
        .unwrap();
    f.state
        .sweep_agent_lifecycle()
        .await
        .expect("old unloaded Room must not stop current Room sweep and owner visibility");
    assert_eq!(
        store.agent_tasks(Some("unloaded-room"), None).unwrap(),
        before,
        "unavailable Room obligations remain durable without guessed completion or replay"
    );
    let current = store.agent_tasks(Some(&f.session), None).unwrap().remove(0);
    assert_eq!(current.state, ExecutionState::Blocked);
    assert!(f.active_ids().contains(&format!(
        "task-blocked-{}-{}",
        current.task_id, current.blocked_revision
    )));
}

#[tokio::test]
async fn a02_owner_resume_failure_is_visible_and_cannot_leave_idle_working() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
    for stale in [false, true] {
        let f = Fixture::new();
        let store = &f.state.owned.durable_state_store;
        let task_id = f.id("task");
        store
            .agent_lifecycle(Operation::Begin {
                owner: DEFAULT_LOCAL_USER_ID.into(),
                room: f.session.clone(),
                agent: "deleted-agent".into(),
                prompt: task_id.clone(),
                run: None,
                now: 1,
            })
            .unwrap();
        let Outcome::Task(blocked) = store
            .agent_lifecycle(Operation::Block {
                task: task_id.clone(),
                prompt: task_id.clone(),
                reason: "Owner clarification required".into(),
            })
            .unwrap()
        else {
            panic!()
        };
        let interaction = format!("task-blocked-{}-{}", task_id, blocked.blocked_revision);
        f.state
            .ensure_task_owner_interaction(blocked.clone())
            .await
            .unwrap();
        if stale {
            store
                .agent_lifecycle(Operation::OwnerResponse {
                    task: task_id.clone(),
                    revision: blocked.blocked_revision,
                    resume: true,
                    now: 2,
                })
                .unwrap();
            store
                .agent_lifecycle(Operation::Block {
                    task: task_id.clone(),
                    prompt: task_id.clone(),
                    reason: "Newer owner decision required".into(),
                })
                .unwrap();
        }
        f.state
            .owned
            .resolve_runtime_interaction(
                &f.session,
                &interaction,
                "resume",
                None,
                Some(DEFAULT_LOCAL_USER_ID),
                false,
            )
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let history = f
                    .state
                    .owned
                    .operational_history_store
                    .load_session_history_entries(&f.session, Some("deleted-agent"))
                    .unwrap();
                if history
                    .iter()
                    .any(|entry| entry.text.contains("Owner action failed"))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("rejected owner disposition must be visible to every client");
        let task = store
            .agent_tasks(Some(&f.session), Some("deleted-agent"))
            .unwrap()
            .remove(0);
        assert_eq!(task.state, ExecutionState::Blocked);
        if stale {
            assert_eq!(task.reason, "Newer owner decision required");
        } else {
            assert!(task.reason.contains("Owner action failed"));
        }
    }
}

#[tokio::test]
async fn sweep_releases_abandoned_and_expired_decisions_without_resolving_live_ones() {
    let fixture = Fixture::new();
    drop(fixture.register("abandoned").unwrap());
    let mut expired = fixture.register("expired").unwrap();
    let mut live = fixture.register("live").unwrap();
    fixture
        .state
        .owned
        .pending_interactions
        .write()
        .get_mut(&fixture.id("expired"))
        .unwrap()
        .kernel_operation_deadline = Some(std::time::Instant::now());
    fixture
        .state
        .owned
        .sweep_kernel_operation_interactions(false);
    assert_eq!(fixture.active_ids(), vec![fixture.id("live")]);
    let result = expired.try_recv().unwrap();
    assert_eq!(result.status, "timed_out");
    assert!(result.choice_id.is_none());
    assert!(matches!(
        live.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(!fixture
        .state
        .owned
        .pending_interactions
        .write()
        .contains_key(&fixture.id("abandoned")));
    fixture
        .state
        .owned
        .sweep_kernel_operation_interactions(true);
    let result = live.try_recv().unwrap();
    assert_eq!(result.status, "timed_out");
    assert!(result.choice_id.is_none());
    assert!(fixture.active_ids().is_empty());
}

#[tokio::test]
async fn a_stale_sweep_candidate_cannot_expire_a_reused_interaction_id() {
    let fixture = Fixture::new();
    let old = fixture.register("reused").unwrap();
    let candidate = fixture
        .state
        .owned
        .pending_interactions
        .write()
        .get(&fixture.id("reused"))
        .unwrap()
        .clone();
    drop(old);
    fixture
        .state
        .owned
        .sweep_kernel_operation_interactions(false);
    let mut replacement = fixture.register("reused").unwrap();
    fixture
        .state
        .owned
        .timeout_runtime_interaction_if_current(
            &fixture.session,
            &fixture.id("reused"),
            Some(&candidate),
        )
        .unwrap();
    assert_eq!(fixture.active_ids(), vec![fixture.id("reused")]);
    assert!(matches!(
        replacement.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    fixture
        .state
        .owned
        .timeout_runtime_interaction(&fixture.session, &fixture.id("reused"))
        .unwrap();
    assert_eq!(replacement.try_recv().unwrap().status, "timed_out");
}

#[tokio::test]
async fn failed_projection_removes_only_its_own_registration_and_responder() {
    let fixture = Fixture::new();
    let mut existing = fixture.register("existing").unwrap();
    // This is the real writer health failure, not a projection mock. Hot session
    // insertion precedes the snapshot check; cleanup must undo only that insert.
    fixture
        .state
        .owned
        .durable_state_store
        .fence_writer()
        .unwrap();
    assert!(fixture.register("failed").is_err());
    assert_eq!(fixture.active_ids(), vec![fixture.id("existing")]);
    let pending = fixture.state.owned.pending_interactions.write();
    assert!(!pending.contains_key(&fixture.id("failed")));
    assert!(pending.contains_key(&fixture.id("existing")));
    drop(pending);
    assert!(matches!(
        existing.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn a_persisted_decision_without_a_responder_is_dropped_after_restart() {
    let fixture = Fixture::new();
    let _live = fixture.register("live").unwrap();
    // A decision restored from session state after an unclean stop has no
    // pending responder: nothing can answer it and it would block a retry.
    let orphan = fixture.id("orphan");
    {
        let mut sessions = fixture.state.owned.session_store.write();
        let mut session = sessions.get_session(&fixture.session).unwrap().clone();
        session.add_active_interaction(RuntimeInteraction::for_kernel_operation(
            &orphan,
            &orphan,
            "Install App?",
            "Review this installation",
            vec![RuntimeInteractionChoice::new(
                "allow", "Install", "allow", None,
            )],
        ));
        // Every vault prompt's responder also lives only in the kernel
        // process (unlock, duration, management menu, passphrase change);
        // other interactions are left alone. One agent each: a session holds
        // a single interaction per subject.
        for (agent, id) in [
            "vault-unlock-agent-1-1",
            "vault-unlock-lease-agent-2-1",
            "vault-manage-agent-3-1",
            "vault-passphrase-current-agent-4-1",
            "provider-notice-1",
        ]
        .into_iter()
        .enumerate()
        {
            session.add_active_interaction(RuntimeInteraction::new(
                id,
                format!("agent-{}", agent + 1),
                crate::session::RuntimeInteractionKind::Choice,
                crate::session::RuntimeInteractionLevel::Critical,
                None,
                "Unlock",
                vec![RuntimeInteractionChoice::new(
                    "cancel", "Cancel", "cancel", None,
                )],
                None,
                None,
                None,
            ));
        }
        sessions.restore_session(session);
    }
    // This fixture's store has not swept yet, so its first pass is unthrottled
    // regardless of other kernels' sweeps in this process.
    fixture
        .state
        .owned
        .sweep_kernel_operation_interactions(false);
    assert_eq!(
        fixture.active_ids(),
        vec![fixture.id("live"), "provider-notice-1".to_string()]
    );
}

// Exercise the real export pump and durable save across two sessions, rather
// than just the prompt-session lookup: an owner may save from another terminal.
#[tokio::test]
async fn app_file_export_saved_from_another_session_closes_only_its_presented_prompt() {
    use crate::durable_state::{
        app_file_exports::{FileExport, FileExportCommand},
        app_state::fixture_inbox_installation,
    };
    use crate::local::{LocalDaemonResponse, SaveAppFileExportRequest};

    let fixture = Fixture::new();
    let store = fixture.state.owned.durable_state_store.clone();
    fixture_inbox_installation(&store, DEFAULT_LOCAL_USER_ID);
    let installation = store
        .get_app_installation(DEFAULT_LOCAL_USER_ID, "installed")
        .unwrap();
    let operation = fixture.id("export");
    let prompt = format!("app_file_export_{operation}");
    store
        .app_file_export(FileExportCommand::Create {
            export: FileExport {
                operation_id: operation.clone(),
                owner: DEFAULT_LOCAL_USER_ID.into(),
                installation: "installed".into(),
                generation: installation.generation,
                name: "imported.txt".into(),
                size: 7,
                state: "pending".into(),
                expires_ms: crate::session::unix_epoch_ms() + 60_000,
            },
            contents: b"fixture".to_vec(),
        })
        .unwrap();
    let mut unrelated = fixture.register("unrelated").unwrap();
    fixture
        .state
        .app_file_export_pass(crate::session::unix_epoch_ms())
        .await;
    assert!(fixture.active_ids().contains(&prompt));

    let answering_session = fixture.id("answering-session");
    fixture
        .state
        .owned
        .session_store
        .write()
        .restore_session(RuntimeSession::new(
            answering_session.clone(),
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        ));
    let result = fixture
        .state
        .save_app_file_export(
            DEFAULT_LOCAL_USER_ID.into(),
            SaveAppFileExportRequest {
                session_id: answering_session,
                operation_id: operation.clone(),
            },
        )
        .await;
    assert!(matches!(
        result,
        LocalDaemonResponse::AppFileExport { operation_id, name, contents_base64 }
            if operation_id == operation && name == "imported.txt" && contents_base64 == "Zml4dHVyZQ=="
    ));
    assert_eq!(fixture.active_ids(), vec![fixture.id("unrelated")]);
    assert!(matches!(
        unrelated.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: owner pause keeps controls usable.
#[tokio::test]
async fn a12_owner_paused_goal_withdraws_decision_without_reopening_on_interval() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let f = Fixture::with_room_tools();
    let store = &f.state.owned.durable_state_store;
    let id = f.id("goal");
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "root".into(),
            prompt: id.clone(),
            run: None,
            now: 1,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::StartGoal {
            task: id.clone(),
            owner: DEFAULT_LOCAL_USER_ID.into(),
            objective: "Implement and verify the owner request".into(),
            minutes: 1,
            now: 1,
        })
        .unwrap();
    let Outcome::Task(blocked) = store
        .agent_lifecycle(Operation::Block {
            task: id.clone(),
            prompt: id.clone(),
            reason: "Owner decision required".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    f.state
        .ensure_task_owner_interaction(blocked.clone())
        .await
        .unwrap();
    assert_eq!(
        f.active_ids().len(),
        1,
        "a real decision must request owner action"
    );
    let mut approval = f.register("unrelated-approval").unwrap();
    let attachment = f
        .state
        .owned
        .attachment_store
        .attach(
            &mut f.state.owned.session_store.write(),
            crate::attachment::AttachRequest::new(
                &f.session,
                "owner-terminal",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ),
        )
        .unwrap();
    let response = f
        .state
        .control_goal_prompt(
            &crate::local::SubmitPromptRequest {
                session_id: f.session.clone(),
                attachment_id: attachment.id().into(),
                target_agent_id: Some("root".into()),
                prompt: format!(
                    "/goal pause {} {}",
                    blocked.goal.as_ref().unwrap().id,
                    blocked.revision
                ),
                attachments: vec![],
            },
            DEFAULT_LOCAL_USER_ID,
        )
        .await
        .unwrap();
    let Some(LocalDaemonResponse::AgentTaskUpdated { task: paused, .. }) = response else {
        panic!()
    };
    let paused = *paused;
    assert_eq!(f.active_ids(), vec![f.id("unrelated-approval")], "normal owner Pause must withdraw the Goal decision immediately, preserving unrelated approval");
    f.state
        .ensure_task_owner_interaction(paused.clone())
        .await
        .unwrap();
    assert!(
        f.active_ids() == vec![f.id("unrelated-approval")],
        "explicit owner pause must withdraw its old decision and not open another modal"
    );
    let mut current = paused;
    for minutes in ["2", "1", "60"] {
        let Outcome::Task(updated) = store
            .agent_lifecycle(Operation::ControlGoal {
                task: id.clone(),
                owner: DEFAULT_LOCAL_USER_ID.into(),
                revision: current.revision,
                action: "interval".into(),
                value: Some(minutes.into()),
                now: 3,
            })
            .unwrap()
        else {
            panic!()
        };
        f.state
            .ensure_task_owner_interaction(updated.clone())
            .await
            .unwrap();
        f.state.sweep_agent_lifecycle().await.unwrap();
        assert!(
            f.active_ids() == vec![f.id("unrelated-approval")],
            "interval edits and sweeps must not reopen an owner-paused Goal modal"
        );
        assert!(updated.goal.as_ref().unwrap().paused);
        current = updated;
    }
    let Outcome::Task(resumed) = store
        .agent_lifecycle(Operation::ControlGoal {
            task: id.clone(),
            owner: DEFAULT_LOCAL_USER_ID.into(),
            revision: current.revision,
            action: "resume".into(),
            value: None,
            now: 4,
        })
        .unwrap()
    else {
        panic!()
    };
    let next_prompt = resumed.pending_prompt_id.unwrap();
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "root".into(),
            prompt: next_prompt.clone(),
            run: None,
            now: 4,
        })
        .unwrap();
    let Outcome::Task(decision) = store
        .agent_lifecycle(Operation::Block {
            task: id.clone(),
            prompt: next_prompt,
            reason: "New owner decision required".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    f.state
        .ensure_task_owner_interaction(decision)
        .await
        .unwrap();
    assert_eq!(
        f.active_ids().len(),
        2,
        "a new genuine decision after Resume must remain visible"
    );
    assert!(matches!(
        approval.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}

// MP-08/MP-10/MP-11: takeover is a wait, never a modal over Release.
#[tokio::test]
async fn computer_human_control_wait_does_not_open_an_owner_modal() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    use crate::session::{CanonicalViewport, EnvironmentActor, EnvironmentActorKind, InputTarget};
    let f = Fixture::with_room_tools();
    f.state
        .start_room_environment(
            &f.session,
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .unwrap();
    f.state
        .owned
        .session_store
        .write()
        .transition_room_environment(&f.session, crate::session::EnvironmentLifecycle::Ready)
        .unwrap();
    f.state
        .request_room_environment_takeover_as_actor(
            &f.session,
            EnvironmentActor::new("human-view", EnvironmentActorKind::Human, "Human"),
            InputTarget::Desktop,
        )
        .unwrap();
    let store = &f.state.owned.durable_state_store;
    let id = f.id("computer-wait");
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "root".into(),
            prompt: id.clone(),
            run: None,
            now: 1,
        })
        .unwrap();
    let Outcome::Task(blocked) = store.agent_lifecycle(Operation::Block {task:id.clone(),prompt:id,reason:"Computer input stopped because a human has control of the desktop; waiting for Release".into()}).unwrap() else {panic!()};
    f.state
        .ensure_task_owner_interaction(blocked.clone())
        .await
        .unwrap();
    assert!(
        f.active_ids().is_empty(),
        "a human-control wait must keep Release clickable"
    );
    f.state
        .ensure_task_owner_interaction(blocked)
        .await
        .unwrap();
    assert!(
        f.active_ids().is_empty(),
        "the periodic sweep must not reopen a modal"
    );
}

// MP-08/MP-10/MP-11: takeover is a wait, never a modal over Release.
#[tokio::test]
async fn computer_release_before_blocked_event_does_not_open_an_owner_modal() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    use crate::session::{CanonicalViewport, EnvironmentActor, EnvironmentActorKind, InputTarget};
    let f = Fixture::with_room_tools();
    f.state
        .start_room_environment(
            &f.session,
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .unwrap();
    f.state
        .owned
        .session_store
        .write()
        .transition_room_environment(&f.session, crate::session::EnvironmentLifecycle::Ready)
        .unwrap();
    f.state
        .request_room_environment_takeover_as_actor(
            &f.session,
            EnvironmentActor::new("human-view", EnvironmentActorKind::Human, "Human"),
            InputTarget::Desktop,
        )
        .unwrap();
    // The human released after the tool refusal, before the provider's Blocked event.
    f.state
        .release_room_environment_input(&f.session, "human-view", &InputTarget::Desktop)
        .unwrap();
    let store = &f.state.owned.durable_state_store;
    let id = f.id("computer-wait");
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "root".into(),
            prompt: id.clone(),
            run: None,
            now: 1,
        })
        .unwrap();
    let Outcome::Task(blocked) = store.agent_lifecycle(Operation::Block {task:id.clone(),prompt:id,reason:"Computer input stopped because a human has control of the desktop; waiting for Release".into()}).unwrap() else {panic!()};
    f.state
        .ensure_task_owner_interaction(blocked.clone())
        .await
        .unwrap();
    assert!(
        f.active_ids().is_empty(),
        "a human-control wait must keep Release clickable"
    );
}

// MP-08/MP-10/MP-11: takeover is a wait, never a modal over Release.
#[tokio::test]
async fn computer_task_decision_leaves_release_accessible_until_human_releases() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    use crate::session::{CanonicalViewport, EnvironmentActor, EnvironmentActorKind, InputTarget};
    let f = Fixture::with_room_tools();
    f.state
        .start_room_environment(
            &f.session,
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .unwrap();
    f.state
        .owned
        .session_store
        .write()
        .transition_room_environment(&f.session, crate::session::EnvironmentLifecycle::Ready)
        .unwrap();
    f.state
        .request_room_environment_takeover_as_actor(
            &f.session,
            EnvironmentActor::new("human-view", EnvironmentActorKind::Human, "Human"),
            InputTarget::Desktop,
        )
        .unwrap();
    let store = &f.state.owned.durable_state_store;
    let id = f.id("computer-wait");
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: "root".into(),
            prompt: id.clone(),
            run: None,
            now: 1,
        })
        .unwrap();
    let Outcome::Task(blocked) = store
        .agent_lifecycle(Operation::Block {
            task: id.clone(),
            prompt: id,
            reason: "Goal evaluation timed out; inspect the receipt and choose whether to resume"
                .into(),
        })
        .unwrap()
    else {
        panic!()
    };
    f.state
        .ensure_task_owner_interaction(blocked.clone())
        .await
        .unwrap();
    assert!(
        f.active_ids().is_empty(),
        "a human-control wait must keep Release clickable"
    );
    f.state
        .release_room_environment_input(&f.session, "human-view", &InputTarget::Desktop)
        .unwrap();
    f.state
        .ensure_task_owner_interaction(blocked)
        .await
        .unwrap();
    assert_eq!(
        f.active_ids().len(),
        1,
        "genuine decisions remain available after Release"
    );
}

// MP-08/MP-09/MP-10/MP-11 A12-S04: the native decision owns the goal wait.
fn native_goal_agent(fixture: &Fixture) -> String {
    fixture
        .state
        .owned
        .agent_store
        .create_agent_for_session(
            crate::agent::CreateAgentRequest::new(&fixture.session, "codex"),
            &fixture
                .state
                .owned
                .session_store
                .get_session(&fixture.session)
                .unwrap(),
        )
        .unwrap()
        .id()
        .to_string()
}

fn native_goal_permission(agent: &str, id: &str) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent,
        crate::session::RuntimeInteractionKind::Permission,
        crate::session::RuntimeInteractionLevel::Warning,
        Some("Command approval required".into()),
        "Run the harmless approval probe",
        vec![
            RuntimeInteractionChoice::new("allow", "Allow once", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        Some(600),
        Some("deny".into()),
    )
}

fn start_native_goal(fixture: &Fixture, agent: &str) -> String {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::session::DEFAULT_LOCAL_USER_ID;
    let store = &fixture.state.owned.durable_state_store;
    let task_id = format!("native-goal-{}", fixture.session);
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: fixture.session.clone(),
            agent: agent.into(),
            prompt: task_id.clone(),
            run: None,
            now: 1,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::StartGoal {
            task: task_id.clone(),
            owner: DEFAULT_LOCAL_USER_ID.into(),
            objective: "Run the harmless approval probe".into(),
            minutes: 1,
            now: 2,
        })
        .unwrap();
    task_id
}

#[tokio::test]
async fn a12_native_approval_blocks_goal_and_answer_resumes_same_turn() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    use crate::session::DEFAULT_LOCAL_USER_ID;
    let fixture = Fixture::with_room_tools();
    let state = &fixture.state;
    let session = fixture.session.clone();
    let agent = native_goal_agent(&fixture);
    let permission = |id: &str| native_goal_permission(&agent, id);
    let store = &state.owned.durable_state_store;
    let task_id = start_native_goal(&fixture, &agent);
    let read_task = || {
        store
            .agent_tasks(Some(&session), Some(&agent))
            .unwrap()
            .remove(0)
    };
    let original = read_task();
    let id = format!("native-goal-approval-{session}");
    let mut receiver = state
        .create_runtime_interaction(&session, permission(&id))
        .await
        .unwrap();
    let blocked = read_task();
    assert_eq!(
        blocked.state,
        ExecutionState::Blocked,
        "unanswered native approval requires owner action"
    );
    assert_eq!(
        blocked.goal.as_ref().unwrap().id,
        original.goal.as_ref().unwrap().id
    );
    // A settings edit advances the task revision without answering or replacing the decision.
    store
        .agent_lifecycle(Operation::ControlGoal {
            task: task_id.clone(),
            owner: DEFAULT_LOCAL_USER_ID.into(),
            revision: blocked.revision,
            action: "interval".into(),
            value: Some("2".into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    assert_eq!(read_task().state, ExecutionState::Blocked);
    state
        .ensure_task_owner_interaction(blocked.clone())
        .await
        .unwrap();
    assert_eq!(
        state
            .owned
            .session_store
            .get_session(&session)
            .unwrap()
            .active_interactions()
            .len(),
        1,
        "the native approval is the only owner decision; do not add a generic Resume card"
    );
    state
        .resolve_runtime_interaction(&session, &id, "allow", None)
        .await
        .unwrap();
    assert_eq!(receiver.try_recv().unwrap().status, "answered");
    let resumed = read_task();
    assert_eq!(resumed.state, ExecutionState::Working);
    assert_eq!(
        resumed.prompt_id, original.prompt_id,
        "continue the pending provider turn, never launch another"
    );
    assert_eq!(resumed.pending_prompt_id, original.pending_prompt_id);
    assert_eq!(
        resumed.goal.as_ref().unwrap().id,
        original.goal.as_ref().unwrap().id
    );
    assert!(resumed.revision > blocked.revision);
    // A later lifecycle change must win over an answer to this approval.
    let second_id = format!("native-goal-second-{session}");
    let _second = state
        .create_runtime_interaction(&session, permission(&second_id))
        .await
        .unwrap();
    store
        .agent_lifecycle(Operation::ControlGoal {
            task: task_id.clone(),
            owner: DEFAULT_LOCAL_USER_ID.into(),
            revision: read_task().revision,
            action: "pause".into(),
            value: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    state
        .resolve_runtime_interaction(&session, &second_id, "allow", None)
        .await
        .unwrap();
    let later = read_task();
    assert_eq!(later.state, ExecutionState::Blocked);
    assert!(later.goal.as_ref().unwrap().paused);
    assert_eq!(
        later.reason,
        "Goal paused by owner; obligations remain supervised"
    );
}

// MP-08/MP-09/MP-10/MP-11 A12-S04: an approval that expires hands the goal back with the turn.
#[tokio::test]
async fn a12_native_approval_timeout_restores_the_goal() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    use crate::session::DEFAULT_LOCAL_USER_ID;
    let fixture = Fixture::with_room_tools();
    let state = &fixture.state;
    let session = fixture.session.clone();
    let agent = native_goal_agent(&fixture);
    let store = &state.owned.durable_state_store;
    let task_id = start_native_goal(&fixture, &agent);
    let read_task = || {
        store
            .agent_tasks(Some(&session), Some(&agent))
            .unwrap()
            .remove(0)
    };
    let original = read_task();
    let id = format!("native-goal-timeout-{session}");
    let mut receiver = state
        .create_runtime_interaction(&session, native_goal_permission(&agent, &id))
        .await
        .unwrap();
    assert_eq!(read_task().state, ExecutionState::Blocked);
    state
        .timeout_runtime_interaction(&session, &id)
        .await
        .unwrap();
    let resolution = receiver.try_recv().unwrap();
    assert_eq!(resolution.status, "answered");
    assert_eq!(resolution.choice_id.as_deref(), Some("deny"));
    let resumed = read_task();
    assert_eq!(
        resumed.state,
        ExecutionState::Working,
        "the provider continues the same turn after the default reply"
    );
    assert_eq!(resumed.prompt_id, original.prompt_id);
    assert_eq!(resumed.pending_prompt_id, original.pending_prompt_id);
    // A later owner pause wins over the expiring approval.
    let second = format!("native-goal-timeout-second-{session}");
    let _receiver = state
        .create_runtime_interaction(&session, native_goal_permission(&agent, &second))
        .await
        .unwrap();
    store
        .agent_lifecycle(Operation::ControlGoal {
            task: task_id,
            owner: DEFAULT_LOCAL_USER_ID.into(),
            revision: read_task().revision,
            action: "pause".into(),
            value: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    state
        .timeout_runtime_interaction(&session, &second)
        .await
        .unwrap();
    let later = read_task();
    assert_eq!(later.state, ExecutionState::Blocked);
    assert!(later.goal.as_ref().unwrap().paused);
}
