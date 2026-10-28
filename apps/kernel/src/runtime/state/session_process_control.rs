//! Kernel-owned process controls share the session's normal collaboration policy.
use super::*;
use crate::runtime::process_resources::{
    ProcessControlResult, ProcessControlTarget, SessionProcessOwner,
};

impl KernelRuntimeState {
    pub(crate) async fn stop_session_processes(
        &self,
        session_id: &str,
        caller: &str,
        target: ProcessControlTarget,
    ) -> Result<ProcessControlResult, DaemonError> {
        self.authorize_current_external_command()?;
        let session = self.owned.session_store.get_session(session_id)?;
        if session.owner_user_id() != caller
            && !session
                .collaboration_level_for_user(caller)
                .is_some_and(|level| level.can_prompt_agent_directly())
        {
            return Err(DaemonError::SessionAccessDenied {
                session_id: session_id.into(),
                user_id: caller.into(),
            });
        }
        let keys = crate::runtime::process_resources::keys_for_session(
            session_id,
            &self.owned.provider_store,
            &self.owned.room_terminals,
        );
        let mut jobs = Vec::new();
        for (owner, key) in keys {
            match &target {
                ProcessControlTarget::Agent { agent_id }
                    if owner == SessionProcessOwner::Agent(agent_id.clone()) =>
                {
                    if crate::runtime::process_resources::has_key(&key) {
                        jobs.push((key, None, owner));
                    }
                }
                ProcessControlTarget::Process { process_id }
                    if crate::runtime::process_resources::samples_for_key(&key)
                        .iter()
                        .any(|row| row.process_id == *process_id) =>
                {
                    jobs.push((key, Some(process_id.clone()), owner))
                }
                _ => {}
            }
        }
        if jobs.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "stop session process",
                message: "the target has ended or is not owned by this session; refresh Health"
                    .into(),
            });
        }
        let mut stopped = Vec::new();
        for (key, process_id, owner) in jobs {
            // Resolve again immediately before dispatch, rather than trusting a
            // client snapshot or a PID that could have changed generations.
            let still_owned = crate::runtime::process_resources::keys_for_session(
                session_id,
                &self.owned.provider_store,
                &self.owned.room_terminals,
            )
            .into_iter()
            .any(|(current_owner, current_key)| current_owner == owner && current_key == key);
            if !still_owned {
                return Err(DaemonError::LocalTransport {
                    operation: "stop session process",
                    message: "process ownership changed; refresh Health".into(),
                });
            }
            let ids = tokio::task::spawn_blocking(move || {
                crate::runtime::process_resources::stop_for_key(&key, process_id)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "stop session process",
                message: error.to_string(),
            })?
            .map_err(|message| DaemonError::LocalTransport {
                operation: "stop session process",
                message,
            })?;
            if let SessionProcessOwner::Agent(agent_id) = owner {
                self.owned.record_notice_for_agent(session_id, None, Some(&agent_id), self.owned.attachment_store.list_session_attachment_ids(session_id), format!("Process outside end: {} owned process(es) stopped by a session controller.", ids.len()));
            }
            stopped.extend(ids);
        }
        Ok(ProcessControlResult {
            outcome: crate::runtime::process_resources::ProcessControlOutcome::OutsideEnd,
            process_ids: stopped,
        })
    }
}
