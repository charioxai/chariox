//! Runtime context lookup for capability execution.

use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

#[derive(Debug, Clone)]
pub(super) struct CapabilityContext {
    pub(super) session_id: String,
    pub(super) attachment_id: String,
    pub(super) workspace_id: String,
    pub(super) worktree_root: PathBuf,
    pub(super) workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    pub(super) operational_history_store: crate::history::OperationalHistoryStore,
    pub(super) operational_artifact_root: PathBuf,
    pub(super) operational_artifact_index_path: PathBuf,
    pub(super) history_archive_enabled: bool,
}

impl CapabilityContext {
    pub(super) fn artifact_root(&self, category: &str) -> PathBuf {
        DaemonApp::attachment_artifact_root(&self.session_id, &self.attachment_id, category)
    }
}

#[derive(Clone)]
pub(crate) struct CapabilityRuntimeStore {
    state: KernelRuntimeState,
}

impl CapabilityRuntimeStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    pub(super) async fn context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityContext, DaemonError> {
        let snapshot = self
            .state
            .capability_context(session_id, attachment_id, capability)
            .await?;
        Ok(CapabilityContext {
            session_id: session_id.to_string(),
            attachment_id: attachment_id.to_string(),
            workspace_id: snapshot.workspace_id,
            worktree_root: snapshot.worktree_root,
            workspace_coordinator: snapshot.workspace_coordinator,
            operational_history_store: snapshot.operational_history_store,
            operational_artifact_root: snapshot.operational_artifact_root,
            operational_artifact_index_path: snapshot.operational_artifact_index_path,
            history_archive_enabled: snapshot.history_archive_enabled,
        })
    }
}
