//! The pane feed is read through the requests a client of the Room may make, so it shows
//! exactly what that caller may see.
use crate::error::DaemonError;
use crate::local::{
    GetPaneFeedRequest, GetPersonInboxRequest, GetSessionStateRequest,
    ListProviderAccountProfilesRequest, ListWorkflowRunsRequest, ListWorkflowsRequest,
    LocalDaemonRequest, LocalDaemonResponse, ReportPaneSettingsRequest,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::pane_feed::{project_pane_feed, PaneFeedSources};

use super::CommandRouter;

/// The page size of the workflow runs a pane feed reads, newest first, until none is left.
const PANE_WORKFLOW_RUNS: u32 = 200;

impl CommandRouter {
    pub(super) async fn get_pane_feed(
        &self,
        command: &KernelCommand,
        request: GetPaneFeedRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let state = self
            .session_view_component(
                command,
                "pane-state",
                LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                    session_id: request.session_id.clone(),
                }),
            )
            .await?;
        let LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            processes,
            ..
        } = state
        else {
            return Err(unexpected_response());
        };
        // The Room state carries no workflow details; the stripes of workflows and their runs are read
        // through the same requests a client of the Room makes.
        let workflows = match self
            .session_view_component(
                command,
                "pane-workflows",
                LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
                    session_id: request.session_id.clone(),
                }),
            )
            .await
        {
            Ok(LocalDaemonResponse::WorkflowsListed { workflows }) => workflows,
            Ok(_) => return Err(unexpected_response()),
            Err(DaemonError::OwnershipAccessDenied { .. }) => Vec::new(),
            Err(error) => return Err(error),
        };
        // Every page: an older run that still goes, or another workflow's latest failed run, may sit
        // behind any number of newer runs of a busy workflow.
        let mut workflow_runs = Vec::new();
        let mut cursor = None;
        loop {
            match self
                .session_view_component(
                    command,
                    "pane-workflow-runs",
                    LocalDaemonRequest::ListWorkflowRuns(ListWorkflowRunsRequest {
                        session_id: request.session_id.clone(),
                        workflow_ref: None,
                        cursor: cursor.take(),
                        limit: Some(PANE_WORKFLOW_RUNS),
                    }),
                )
                .await
            {
                Ok(LocalDaemonResponse::WorkflowRunsListed {
                    workflow_runs: page,
                    next_cursor,
                }) => {
                    workflow_runs.extend(page);
                    match next_cursor {
                        Some(next) => cursor = Some(next),
                        None => break,
                    }
                }
                Ok(_) => return Err(unexpected_response()),
                Err(DaemonError::OwnershipAccessDenied { .. }) => {
                    workflow_runs.clear();
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        let session = session.with_room_workflow_details(workflows, workflow_runs);
        let inbox = self
            .session_view_component(
                command,
                "pane-inbox",
                LocalDaemonRequest::GetPersonInbox(GetPersonInboxRequest),
            )
            .await?;
        let LocalDaemonResponse::PersonInbox { cards, .. } = inbox else {
            return Err(unexpected_response());
        };
        // A Room collaborator sees none of the owner's accounts: their feed names no account.
        let accounts = match self
            .session_view_component(
                command,
                "pane-accounts",
                LocalDaemonRequest::ListProviderAccountProfiles(
                    ListProviderAccountProfilesRequest { provider: None },
                ),
            )
            .await
        {
            Ok(LocalDaemonResponse::ProviderAccountProfilesListed { profiles }) => profiles,
            Ok(_) => return Err(unexpected_response()),
            Err(DaemonError::OwnershipAccessDenied { .. }) => Vec::new(),
            Err(error) => return Err(error),
        };
        let cloud_origin = self
            .runtime_state
            .config_snapshot()
            .await
            .cloud_relay
            .and_then(|profile| {
                crate::transport::relay_client::paired_cloud_origin(&profile.api_url)
            });
        let feed = project_pane_feed(
            &PaneFeedSources {
                session: &session,
                activity: &agent_activity,
                processes: &processes,
                accounts: &accounts,
                inbox: &cards,
                cloud_origin: cloud_origin.as_deref(),
            },
            &request.agent_id,
        )?;
        Ok(LocalDaemonResponse::PaneFeed { feed })
    }

    pub(super) async fn report_pane_settings(
        &self,
        command: &KernelCommand,
        request: ReportPaneSettingsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let feed_request = GetPaneFeedRequest {
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
        };
        self.runtime_state
            .report_pane_settings(request, &command_caller_user_id(command))
            .await?;
        self.get_pane_feed(command, feed_request).await
    }
}

fn unexpected_response() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "read pane feed",
        message: "the kernel answered a pane feed read with another response".into(),
    }
}
