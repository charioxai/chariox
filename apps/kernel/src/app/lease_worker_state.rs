use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::write_private_file;
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent};
use crate::transport::relay_client::LeaseCallerAuthorization;

use super::DaemonApp;

const LEASE_WORKER_STATE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct LeaseWorkerState {
    version: u32,
    kernel_id: String,
    execution_leases: BTreeMap<String, ExecutionLease>,
    leased_agents: BTreeMap<String, LeasedAgent>,
    callers: LeaseCallerAuthorization,
}

impl DaemonApp {
    fn lease_worker_state_path(&self) -> std::path::PathBuf {
        self.config
            .private_runtime_state_root()
            .join("lease-worker-state.json")
    }

    pub(super) fn restore_lease_worker_state(&mut self) -> Result<(), DaemonError> {
        if self.config.lease_worker_capacity.is_none() {
            return Ok(());
        }
        let bytes = match fs::read(self.lease_worker_state_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(state_error("read", error.to_string())),
        };
        let mut state: LeaseWorkerState = serde_json::from_slice(&bytes)
            .map_err(|error| state_error("decode", error.to_string()))?;
        if state.version != LEASE_WORKER_STATE_VERSION
            || state.kernel_id != self.config.daemon_id
            || state.execution_leases.len()
                > self.config.lease_worker_capacity.unwrap_or(0) as usize
            || state.execution_leases.iter().any(|(id, lease)| {
                id != &lease.id || lease.worker_kernel_id != self.config.daemon_id
            })
            || state
                .leased_agents
                .iter()
                .any(|(id, agent)| id != &agent.id)
            || !state
                .callers
                .matches_runtime_state(&state.execution_leases, &state.leased_agents)
        {
            return Err(state_error(
                "validate",
                "lease worker state is incompatible",
            ));
        }
        for agent in state.leased_agents.values_mut() {
            let lease = state
                .execution_leases
                .get(&agent.lease_id)
                .ok_or_else(|| state_error("validate", "leased agent has no reservation"))?;
            let session = self
                .sessions
                .get_session(&agent.backing_session_id)
                .map_err(|error| state_error("validate", error.to_string()))?;
            let backing_agent = self
                .agents
                .get_agent(&agent.backing_agent_id)
                .map_err(|error| state_error("validate", error.to_string()))?;
            if !session.is_hidden()
                || session.owner_user_id() != lease.owner_user_id
                || backing_agent.session_id() != session.id()
                || backing_agent.owner_user_id() != lease.owner_user_id
            {
                return Err(state_error(
                    "validate",
                    "leased backing runtime is incompatible",
                ));
            }
            let attachment = self.attachments.attach(
                &mut self.sessions.write(),
                AttachRequest::for_user(
                    session.id(),
                    format!("leased-agent:{}", lease.home_agent_id),
                    ClientCapabilityLevel::MessageTransport,
                    lease.owner_user_id.clone(),
                ),
            )?;
            agent.backing_attachment_id = attachment.id().to_string();
        }
        let callers = state.callers;
        let mut relay_state = self
            .relay_client_state
            .try_write()
            .map_err(|error| state_error("restore", error.to_string()))?;
        relay_state.restore_lease_callers(callers.clone());
        drop(relay_state);
        self.execution_leases = state.execution_leases;
        self.leased_agents = state.leased_agents;
        self.persist_lease_worker_state(&callers)?;
        Ok(())
    }

    pub(crate) fn persist_lease_worker_state(
        &self,
        callers: &LeaseCallerAuthorization,
    ) -> Result<(), DaemonError> {
        if self.config.lease_worker_capacity.is_none() {
            return Ok(());
        }
        if !callers.matches_runtime_state(&self.execution_leases, &self.leased_agents) {
            return Err(state_error(
                "validate",
                "lease caller state does not match runtime",
            ));
        }
        let state = LeaseWorkerState {
            version: LEASE_WORKER_STATE_VERSION,
            kernel_id: self.config.daemon_id.clone(),
            execution_leases: self.execution_leases.clone(),
            leased_agents: self.leased_agents.clone(),
            callers: callers.clone(),
        };
        let bytes =
            serde_json::to_vec(&state).map_err(|error| state_error("encode", error.to_string()))?;
        write_private_file(&self.lease_worker_state_path(), &bytes)
            .map_err(|error| state_error("write", error.to_string()))
    }
}

fn state_error(operation: &'static str, message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "lease_worker_state",
        message: format!("{operation}: {}", message.into()),
    }
}
