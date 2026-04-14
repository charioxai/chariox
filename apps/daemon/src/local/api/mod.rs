use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;
use crate::app::{DaemonApp, SessionHistoryCursor, SessionHistoryPageEntry};
use crate::attachment::{ClientCapabilityLevel, RuntimeAttachment};
use crate::capability::{
    CaptureScreenshotResult, EditFileResult, InspectGitResult, ReadDirectoryTreeResult,
    ReadFileResult, RunShellCommandResult, StoredTransferArtifact,
};
use crate::error::DaemonError;
use crate::kernel::projection::DaemonHealthProjection;
use crate::provider::{
    OpenCodeProviderCatalog, ProviderAuthStatus, ProviderCommandCatalog, ProviderLoginStart,
    ProviderProcessInfo, RuntimeProviderRun,
};
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion,
    PromptSubmissionOutcome, QueuedWorkflowLaunch, RuntimeSession, SessionConfigState,
    WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowLaunchPolicy,
    WorkflowNodeDefinition, WorkflowRun, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
};

#[cfg(test)]
mod tests;
mod types;

pub use types::*;

impl DaemonApp {
    pub(crate) fn local_api_session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let mut session = self.sessions().get_session(session_id)?;
        let agents = self.agents().get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.update_session_projection(session.clone());
        Ok(session)
    }

    pub(crate) fn list_sessions_response(&self) -> Result<LocalDaemonResponse, DaemonError> {
        let sessions = self.sessions().list_sessions();
        let sessions_with_agents: Vec<_> = sessions
            .into_iter()
            .map(|mut session| {
                let agents = self.agents().get_session_agents(session.id());
                session.set_agents(agents);
                session
            })
            .collect();
        Ok(LocalDaemonResponse::SessionsListed {
            sessions: sessions_with_agents,
        })
    }

    pub(crate) fn resolve_session_response(
        &self,
        request: ResolveSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut session =
            self.resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?;
        let agents = self.agents().get_session_agents(session.id());
        session.set_agents(agents);
        Ok(LocalDaemonResponse::SessionResolved { session })
    }

    pub(crate) fn get_session_state_response(
        &self,
        request: GetSessionStateRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::SessionState { session })
    }

    pub(crate) fn list_agents_response(
        &self,
        request: ListAgentsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agents = self.list_session_agents(&request.session_id);
        Ok(LocalDaemonResponse::AgentsListed { agents })
    }
}
