use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::session::MetaagentTaskStatus;

use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) fn start_metaagent_task_for_prompt(
        &self,
        session_id: &str,
        metaagent_id: &str,
        prompt: &str,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        let agent = self.owned.agent_store.get_agent(metaagent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "metaagent_task_request",
                message: format!("agent `{metaagent_id}` is not in this session"),
            });
        }
        if !agent.is_metaagent() {
            return Ok(None);
        }
        let Some(session) = self
            .owned
            .session_store
            .write()
            .start_metaagent_task_if_needed(session_id, metaagent_id, prompt)?
        else {
            return Ok(None);
        };
        Ok(Some(self.project_metaagent_task_session(session)))
    }

    pub(crate) async fn execute_metaagent_task_request(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::UpdateMetaagentTask(request) => {
                self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                if request.task_markdown.is_none() && request.plan_markdown.is_none() {
                    return Err(DaemonError::LocalTransport {
                        operation: "update_metaagent_task",
                        message: "task_markdown or plan_markdown is required".to_string(),
                    });
                }
                let mut sessions = self.owned.session_store.write();
                let mut session = if let Some(task_markdown) = request.task_markdown {
                    sessions.update_metaagent_task_markdown(
                        &request.session_id,
                        &request.metaagent_id,
                        task_markdown,
                    )?
                } else {
                    sessions.get_session(&request.session_id)?
                };
                if let Some(plan_markdown) = request.plan_markdown {
                    session = sessions.update_metaagent_plan_markdown(
                        &request.session_id,
                        &request.metaagent_id,
                        plan_markdown,
                    )?;
                }
                drop(sessions);
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            LocalDaemonRequest::PauseMetaagentTask(request) => {
                self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                let session = self.owned.session_store.write().set_metaagent_task_status(
                    &request.session_id,
                    &request.metaagent_id,
                    MetaagentTaskStatus::Paused,
                )?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            LocalDaemonRequest::ResumeMetaagentTask(request) => {
                self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                let session = self.owned.session_store.write().set_metaagent_task_status(
                    &request.session_id,
                    &request.metaagent_id,
                    MetaagentTaskStatus::Active,
                )?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            LocalDaemonRequest::AbortMetaagentTask(request) => {
                self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                let session = self.owned.session_store.write().abort_metaagent_task(
                    &request.session_id,
                    &request.metaagent_id,
                    request.reason,
                )?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "metaagent_task_request",
                message: "unsupported metaagent task request".to_string(),
            }),
        }
    }

    fn ensure_session_metaagent(
        &self,
        session_id: &str,
        metaagent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(metaagent_id)?;
        if agent.session_id() != session_id || !agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "metaagent_task_request",
                message: format!("agent `{metaagent_id}` is not a metaagent in this session"),
            });
        }
        Ok(())
    }

    fn project_metaagent_task_session(
        &self,
        mut session: crate::session::RuntimeSession,
    ) -> crate::session::RuntimeSession {
        let agents = self.owned.agent_store.get_session_agents(session.id());
        session.set_agents(agents);
        self.owned.project_session_runtime_view(&mut session);
        self.owned.session_projection.update(session.clone());
        session
    }
}

fn metaagent_task_response(
    session: crate::session::RuntimeSession,
    metaagent_id: &str,
) -> LocalDaemonResponse {
    let task = session.metaagent_task(metaagent_id).cloned();
    LocalDaemonResponse::MetaagentTaskUpdated { session, task }
}
