use std::collections::BTreeMap;
use std::path::Path;

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::config::WorkflowCodeLimitsConfig;
use crate::error::DaemonError;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::history::SessionHistoryEntry;
use crate::provider::{AgentEndpointMode, ProviderRunState, adapter_key_for_provider};
use crate::session::{
    CreateSessionRequest, RuntimeSession, SessionStateOwner, SessionStateReader, SessionStatus,
};
use crate::workflow_code::{
    WorkflowCodeAgentBinding, WorkflowCodeApplyReport, WorkflowCodeCompileAndApplyResult,
    WorkflowCodeCompileResult, WorkflowCodeDefinition, WorkflowCodeLanguage,
    WorkflowCodeValidationDiagnostic, WorkflowCodeValidationReport, WorkflowCodeValidationSeverity,
    compile_workflow_code_source_with_schema_import_root,
};

pub(crate) struct KernelSessionService<'a> {
    app: &'a mut DaemonApp,
}

mod workflow_code;

#[cfg(test)]
mod tests;

pub(crate) struct KernelSessionReadService<'a> {
    app: &'a DaemonApp,
}

impl<'a> KernelSessionReadService<'a> {
    pub(crate) fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn session_snapshot(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session = self.session_snapshot_without_projection_update(session_id)?;
        self.app.update_session_projection(session.clone());
        Ok(session)
    }

    pub(crate) fn session_snapshot_without_projection_update(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let mut session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let agents = self.app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        self.app.project_session_runtime_view(&mut session);
        Ok(session)
    }

    pub(crate) fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self.app.attachments.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    pub(crate) fn session_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        self.app.load_session_history_entries(&session, None)
    }
}

impl<'a> KernelSessionService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        let session =
            SessionStateOwner::new(self.app.session_state_store()).create_session(request)?;
        let defaults = session.agent_defaults();
        let mut agent_request = CreateAgentRequest::new(session.id(), &defaults.provider)
            .with_owner_user_id(session.owner_user_id().to_string())
            .with_worktree(session.worktree_id());
        if let Some(model) = defaults.model.as_deref() {
            agent_request = agent_request.with_model(model.to_string());
        }
        if let Some(effort) = defaults.effort.as_deref() {
            agent_request = agent_request.with_effort(effort.to_string());
        }
        if let Some(account_profile) = defaults.account_profile.as_deref() {
            agent_request = agent_request.with_account_profile(account_profile.to_string());
        }
        if let Some(execution_mode) = defaults.execution_mode {
            agent_request = agent_request.with_execution_mode_override(execution_mode);
        }
        if let Some(permission_level) = defaults.permission_level {
            agent_request = agent_request.with_permission_level_override(permission_level);
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self.app.agents.create_agent(agent_request, &mut sessions)?;
        drop(sessions);
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session.id())?;
        self.app.durable_state_store().append_event(
            "session.created",
            Some(session.id().to_string()),
            serde_json::json!({
                "session": &session,
                "default_agent": &agent,
            }),
        )?;

        crate::logging::info_with_fields(
            "daemon.session",
            "session created with default agent",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_ref": agent.agent_ref(),
            }),
        );

        Ok((session, agent))
    }

    pub(crate) fn attach(
        &mut self,
        request: AttachRequest,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .app
            .attachments
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let attachment = self.app.attachments.attach(&mut sessions, request)?;
        drop(sessions);

        // Create default agent if session has no agents (e.g., after session was ended and reattached).
        // Parked/active sessions that were never ended will retain their existing agents.
        let session_agents = self.app.agents.get_session_agents(&session_id);
        if session_agents.is_empty() {
            let worktree_id = self
                .app
                .sessions()
                .get_session(&session_id)?
                .worktree_id()
                .to_string();
            let agent_request =
                CreateAgentRequest::new(&session_id, "default").with_worktree(worktree_id);
            let session_store = self.app.session_state_store();
            let mut sessions = session_store.write();
            let _agent = self.app.agents.create_agent(agent_request, &mut sessions)?;
            drop(sessions);
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }

        self.app.sync_focused_provider_run_if_idle(&session_id)?;

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment joined session",
            serde_json::json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "client_id": client_id,
                "capability_level": capability_level,
                "replaced_attachment_ids": replaced_attachment_ids,
            }),
        );
        Ok(attachment)
    }

    pub(crate) fn spawn_agent(
        &mut self,
        mut request: CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        if let Some(kernel_ref) = request.kernel_ref.clone() {
            if self.app.kernel_ref_is_local(&kernel_ref) {
                request.kernel_ref = None;
            } else {
                let agent = self.app.spawn_worker_agent(request, &kernel_ref)?;
                self.app.durable_state_store().append_event(
                    "agent.created",
                    Some(agent.id().to_string()),
                    serde_json::json!({
                        "agent": &agent,
                    }),
                )?;
                return Ok(agent);
            }
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self.app.agents.create_agent(request, &mut sessions)?;
        drop(sessions);
        self.app.durable_state_store().append_event(
            "agent.created",
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
            }),
        )?;
        Ok(agent)
    }

    pub(crate) fn apply_workflow_code_definition(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        self.apply_workflow_code_definition_with_rebindings(
            session_id,
            definition,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
            &[],
        )
    }

    pub(crate) fn apply_workflow_code_definition_with_alias_base(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        alias_base: Option<&str>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        self.apply_workflow_code_definition_with_rebindings_and_alias_base(
            session_id,
            definition,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
            &[],
            alias_base,
        )
    }

    pub(crate) fn apply_workflow_code_definition_with_rebindings(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        agent_rebindings: &[crate::workflow_code::WorkflowCodeAgentRebinding],
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        self.apply_workflow_code_definition_with_rebindings_and_alias_base(
            session_id,
            definition,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            provider_rebindings,
            agent_rebindings,
            None,
        )
    }
}

fn workflow_code_alias_can_allocate(
    session: &crate::session::RuntimeSession,
    requested_alias: Option<&str>,
) -> bool {
    let Some(alias) = requested_alias else {
        return true;
    };
    let trimmed_alias = alias.trim().to_lowercase();
    if trimmed_alias.is_empty() {
        return true;
    }
    (0..crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS).any(|attempt| {
        let candidate_alias = if attempt == 0 {
            trimmed_alias.clone()
        } else {
            format!("{trimmed_alias}-{}", attempt + 1)
        };
        !session
            .workflows()
            .iter()
            .any(|workflow| workflow.alias() == Some(candidate_alias.as_str()))
    })
}

fn push_workflow_code_target_validation_error(
    validation: &mut WorkflowCodeValidationReport,
    code: &'static str,
    message: impl Into<String>,
    handle: Option<String>,
) {
    validation.ok = false;
    validation
        .diagnostics
        .push(WorkflowCodeValidationDiagnostic {
            severity: WorkflowCodeValidationSeverity::Error,
            code: code.to_string(),
            message: message.into(),
            handle,
            source_span: None,
        });
}

fn workflow_code_validation_error_message(
    prefix: &'static str,
    validation: &WorkflowCodeValidationReport,
) -> String {
    let details = validation
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let handle = diagnostic
                .handle
                .as_deref()
                .map(|handle| format!(" handle `{handle}`"))
                .unwrap_or_default();
            format!("{}{}: {}", diagnostic.code, handle, diagnostic.message)
        })
        .collect::<Vec<_>>()
        .join("; ");
    if details.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {details}")
    }
}
