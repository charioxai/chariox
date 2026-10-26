//! Only acknowledged Move backings survive a worker restart. They restore idle,
//! without a provider run, under the original lease expiry and caller identity.
use super::*;
use crate::agent::AgentInstance;
use crate::agent_move::{error as move_error, MoveBacking};

impl DaemonApp {
    pub(crate) fn abort_agent_move_target(
        &mut self,
        identity: &crate::agent_move::AgentMoveIdentity,
    ) -> Result<(), DaemonError> {
        // Lease creation and this tombstone share the app authority mutex.
        // A lost lease reply must never let cancellation restore two executors.
        if self.execution_leases.values().any(|lease| {
            lease.home_kernel_id == identity.home_kernel_id
                && lease.home_session_id == identity.home_session_id
                && lease.home_agent_id == identity.home_agent_id
        }) {
            return Err(move_error(
                "The target already holds a Move lease; finish the operation before returning",
            ));
        }
        let Some(mut target) = self.agent_moves.target(&identity.operation_id) else {
            return Ok(());
        };
        if target.capture.identity != *identity {
            return Err(move_error("Move target generation changed"));
        }
        if target.active || target.backing.is_some() || target.backing_agent_id.is_some() {
            return Err(move_error(
                "The target already holds a Move lease; finish the operation before returning",
            ));
        }
        target.retired = true;
        self.agent_moves.save_target(target)
    }

    pub(crate) fn retain_agent_move_backing(
        &self,
        operation: &str,
        leased_agent_id: &str,
    ) -> Result<(), DaemonError> {
        let mut target = self
            .agent_moves
            .target(operation)
            .ok_or_else(|| move_error("Move target journal is missing"))?;
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| move_error("Move backing agent is missing"))?;
        let lease = self
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| move_error("Move backing lease is missing"))?;
        let session = self
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let agent = self.agents.get_agent(&leased_agent.backing_agent_id)?;
        let attachment = self
            .attachments
            .get_attachment(&leased_agent.backing_attachment_id)?;
        if !target.ready
            || target.retired
            || target.backing_agent_id.as_deref() != Some(agent.id())
            || lease.home_kernel_id != target.capture.identity.home_kernel_id
            || lease.home_session_id != target.capture.identity.home_session_id
            || lease.home_agent_id != target.capture.identity.home_agent_id
            || lease.owner_user_id != agent.owner_user_id()
            || session.owner_user_id() != agent.owner_user_id()
            || attachment.owner_user_id() != agent.owner_user_id()
        {
            return Err(move_error(
                "Move backing does not match its readiness and ownership receipt",
            ));
        }
        target.backing = Some(MoveBacking {
            lease: lease.clone(),
            leased_agent,
            session,
            agent,
            attachment_owner_user_id: attachment.owner_user_id().into(),
            attachment,
            execution_caller: self.execution_lease_callers.get(&lease.id).cloned(),
            agent_caller: self.leased_agent_callers.get(leased_agent_id).cloned(),
        });
        self.agent_moves.save_target(target)
    }

    pub(super) fn restore_agent_move_backings(&mut self) -> Result<(), DaemonError> {
        for target in self.agent_moves.targets() {
            if target.retired {
                continue;
            }
            if let Some(mut project) = target.project.clone() {
                if let Some(event) = self
                    .durable_state
                    .load_subject_events_by_kind(project.id(), "project.updated", 1)?
                    .into_iter()
                    .last()
                {
                    if let Some(value) = event.payload.get("project") {
                        let current: crate::session::RuntimeProject =
                            serde_json::from_value(value.clone())
                                .map_err(|_| move_error("Moved Project state is invalid"))?;
                        if current.id() != project.id()
                            || current.owner_user_id() != project.owner_user_id()
                        {
                            return Err(move_error("Moved Project ownership changed"));
                        }
                        project = current;
                    }
                }
                self.sessions.restore_projects(vec![project]);
            }
            let Some(backing) = target.backing else {
                continue;
            };
            let identity = &target.capture.identity;
            if identity.target_kernel_id != self.config.daemon_id
                || target.backing_session_id.as_deref() != Some(backing.session.id())
                || target.backing_agent_id.as_deref() != Some(backing.agent.id())
                || backing.lease.worker_kernel_id != self.config.daemon_id
                || backing.lease.machine_id != self.config.host_machine_id
                || backing.lease.home_kernel_id != identity.home_kernel_id
                || backing.lease.home_session_id != identity.home_session_id
                || backing.lease.home_agent_id != identity.home_agent_id
                || backing.lease.owner_user_id != backing.session.owner_user_id()
                || backing.agent.owner_user_id() != backing.session.owner_user_id()
                || backing.attachment_owner_user_id != backing.session.owner_user_id()
                || !backing.session.is_hidden()
            {
                return Err(move_error("Durable Move backing identity is invalid; repair kernel state before admitting work"));
            }
            // Native acceptance/profile records are not session.updated checkpoints;
            // read their last state even when a later Room snapshot skipped this
            // deliberately hidden execution Room.
            let mut agent = backing.agent;
            let mut latest = None;
            for kind in ["agent.runtime_profile_updated", "agent.updated"] {
                if let Some(event) = self
                    .durable_state
                    .load_subject_events_by_kind(agent.id(), kind, 1)?
                    .into_iter()
                    .last()
                {
                    if latest.as_ref().is_none_or(
                        |previous: &crate::durable_state::DurableStateEvent| {
                            event.sequence > previous.sequence
                        },
                    ) {
                        latest = Some(event);
                    }
                }
            }
            if let Some(event) = latest {
                if let Some(value) = event.payload.get("agent") {
                    let restored: AgentInstance = serde_json::from_value(value.clone())
                        .map_err(|_| move_error("Moved native identity record is invalid"))?;
                    if restored.id() != agent.id()
                        || restored.session_id() != backing.session.id()
                        || restored.owner_user_id() != backing.session.owner_user_id()
                    {
                        return Err(move_error("Moved native identity record changed ownership"));
                    }
                    agent = restored;
                }
            }
            agent.set_state(crate::agent::AgentState::Idle);
            agent.set_processing(false);
            let session = self.sessions.restore_ephemeral_session(backing.session)?;
            self.prompt_state_owner.restore_session_state(&session);
            self.agents.restore_agent(agent);
            let attachment = crate::attachment::RuntimeAttachment::new(
                backing.attachment.id(),
                session.id(),
                backing.attachment.client_id(),
                backing.attachment.capability_level(),
                &backing.attachment_owner_user_id,
            );
            self.attachments
                .restore_attachment(&mut self.sessions.write(), attachment)?;
            if let Some(caller) = backing.execution_caller {
                self.execution_lease_callers
                    .insert(backing.lease.id.clone(), caller);
            }
            if let Some(caller) = backing.agent_caller {
                self.leased_agent_callers
                    .insert(backing.leased_agent.id.clone(), caller);
            }
            self.execution_leases
                .insert(backing.lease.id.clone(), backing.lease);
            self.leased_agents
                .insert(backing.leased_agent.id.clone(), backing.leased_agent);
        }
        Ok(())
    }
}
