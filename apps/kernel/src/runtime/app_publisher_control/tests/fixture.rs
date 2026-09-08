use super::*;
use crate::{
    app::DaemonApp,
    config::DaemonConfig,
    session::{CollaborationLevel, RuntimeSession},
};
use std::os::unix::fs::PermissionsExt;
pub(super) struct Fixture {
    pub(super) state: KernelRuntimeState,
    pub(super) session: String,
    path: std::path::PathBuf,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let path = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-publisher-control-{:016x}",
                rand::random::<u64>()
            ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(path.join("kernel.sqlite").display().to_string());
        config.session_history_root_default = path.join("history");
        config.user_config.history.operational.path =
            Some(path.join("operational.sqlite").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(path.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(path.join("artifact-index.sqlite").display().to_string());
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let mut session = RuntimeSession::new(
            "publisher-session",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        session.add_member("alice", None, CollaborationLevel::Full);
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
            Arc::new(tokio::sync::Mutex::new(app)),
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
            path,
        }
    }
    pub(super) fn control(&self) -> &AppPublisherControl {
        self.state.app_control().publishers()
    }
    pub(super) fn input(&self) -> PublisherEnrollmentInput {
        PublisherEnrollmentInput {
            session_id: self.session.clone(),
            publisher_id: "local.developer".into(),
            key_id: "development".into(),
            public_key: ed25519_dalek::SigningKey::from_bytes(&[29; 32])
                .verifying_key()
                .to_bytes(),
            expected_revision: 0,
        }
    }
    pub(super) fn phase(&self, request: &str) -> Option<PublisherOperationPhase> {
        self.control()
            .0
            .shared
            .store
            .publisher_enrollment_status(
                "alice",
                request,
                AppOperationBudget::from_supervisor(|| false),
            )
            .ok()
            .map(|v| v.phase)
    }
    pub(super) fn waiting(&self) -> Option<String> {
        let state = self.control().0.state.lock().unwrap();
        state.entries.values().find_map(|e| match &e.step {
            Step::Waiting { challenge, .. } => Some(challenge.interaction_id().to_owned()),
            _ => None,
        })
    }
    pub(super) async fn begin(&self, request: &str) -> PublisherOperation {
        self.control()
            .begin(&self.state, "alice", request, self.input())
            .await
            .unwrap()
    }
    pub(super) async fn until(&self, mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while !condition() {
                self.control().pump(&self.state).await;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("bounded publisher pump");
    }
    pub(super) async fn shutdown(&self) {
        let control = self.control().clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || control.shutdown_blocking(handle))
            .await
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.control().0.shared.store.fence_writer();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
