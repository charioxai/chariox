//! Owner Inbox notices for admissions refused before a workflow run exists.

use super::*;
use crate::durable_state::{StoredNotice, WORKFLOW_RELINK_ORIGIN, WORKFLOW_START_NOTICE_PREFIX};
use crate::local::{InvokeWorkflowEndpointRequest, PersonInboxPriority};
use crate::runtime::command::{caller_user_id, KernelCaller};
use crate::session::DEFAULT_LOCAL_USER_ID;
use sha2::{Digest, Sha256};

const TITLE: &str = "Workflow couldn’t start";
const RELINK: &str = "This kernel is no longer linked to the workflow endpoint owner’s account. Re-link it with /cloud link on this kernel, then approve its device code at /activate while signed in to that account.";
const SWITCH_ACCOUNT: &str =
    "This account cannot start this workflow endpoint. Switch to the endpoint owner’s account and try again.";

impl KernelRuntimeState {
    pub(in crate::runtime) fn can_read_workflow_relink_notice(
        &self,
        caller: &KernelCaller,
    ) -> bool {
        caller.is_terminal_caller() && self.is_unlinked_local_user(&caller_user_id(caller))
    }

    fn is_unlinked_local_user(&self, user: &str) -> bool {
        user == DEFAULT_LOCAL_USER_ID
            && self
                .owned
                .config_projection
                .snapshot()
                .cloud_relay
                .is_none()
    }

    pub(super) fn report_workflow_start_refusal(
        &self,
        request: &InvokeWorkflowEndpointRequest,
        caller: &str,
        error: DaemonError,
    ) -> DaemonError {
        let relink = self.is_unlinked_local_user(caller);
        let action = if relink { RELINK } else { SWITCH_ACCOUNT };
        match self.record_workflow_start_alert(request, caller, relink) {
            Ok(()) => self.record_person_inbox_change(),
            Err(error) => crate::logging::warn_with_fields(
                "workflow.start_alert",
                "could not update the workflow start alert",
                serde_json::json!({ "error": error.to_string() }),
            ),
        }
        DaemonError::WorkflowStartRefused {
            source: Box::new(error),
            action,
        }
    }

    fn record_workflow_start_alert(
        &self,
        request: &InvokeWorkflowEndpointRequest,
        caller: &str,
        relink: bool,
    ) -> Result<(), DaemonError> {
        let notice = {
            let sessions = self.owned.session_store.read();
            let room = sessions.get_session(&request.session_id)?;
            let workflow = sessions.resolve_workflow_ref(room.id(), &request.workflow_ref)?;
            let endpoint = sessions.resolve_workflow_endpoint_ref(
                room.id(),
                workflow.id(),
                &request.endpoint_ref,
            )?;
            // The local operator can repair the link without access to the Cloud-owned Room.
            let (owner, agent, body) = if relink {
                (
                    DEFAULT_LOCAL_USER_ID,
                    WORKFLOW_RELINK_ORIGIN,
                    RELINK.to_owned(),
                )
            } else {
                let node = workflow.node(endpoint.entry_node_id()).ok_or_else(|| {
                    DaemonError::WorkflowNodeNotFound {
                        session_id: room.id().to_owned(),
                        workflow_id: workflow.id().to_owned(),
                        node_id: endpoint.entry_node_id().to_owned(),
                    }
                })?;
                let name =
                    crate::durable_state::workflow_notifications::naming::workflow_name(&workflow);
                (
                    room.owner_user_id(),
                    node.agent_id(),
                    format!("{name}. {SWITCH_ACCOUNT}"),
                )
            };
            let prefix = endpoint_notice_prefix(room.id(), endpoint.id());
            StoredNotice {
                card_id: format!("{prefix}{:x}", Sha256::digest(caller.as_bytes())),
                owner_user_id: owner.to_owned(),
                session_id: room.id().to_owned(),
                agent_id: agent.to_owned(),
                title: TITLE.to_owned(),
                body: Some(body),
                link: None,
                priority: PersonInboxPriority::High,
                created_at_ms: crate::session::unix_epoch_ms(),
                read: false,
                folded: 0,
            }
        };
        self.durable_state_store()
            .record_workflow_start_refusal(notice)
    }

    pub(super) fn clear_workflow_start_alerts(&self, room: &str, endpoint: &str) {
        match self
            .durable_state_store()
            .clear_workflow_start_refusals(&endpoint_notice_prefix(room, endpoint))
        {
            Ok(true) => self.record_person_inbox_change(),
            Ok(false) => {}
            Err(error) => crate::logging::warn_with_fields(
                "workflow.start_alert",
                "could not clear the workflow start alert",
                serde_json::json!({ "error": error.to_string() }),
            ),
        }
    }
}

fn endpoint_notice_prefix(room: &str, endpoint: &str) -> String {
    let identity = serde_json::json!([room, endpoint]).to_string();
    format!(
        "{WORKFLOW_START_NOTICE_PREFIX}{:x}:",
        Sha256::digest(identity.as_bytes())
    )
}
