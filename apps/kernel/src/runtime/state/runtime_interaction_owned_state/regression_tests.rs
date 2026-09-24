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
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
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
                session_id: "same-session".into(),
                session_store_identity: sessions.weak_identity(),
                kernel_operation_owner: owner,
                kernel_operation_deadline: None,
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
