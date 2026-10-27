use std::path::PathBuf;
use std::time::Duration;

use super::*;
use crate::local::{
    LocalDaemonResponse, RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult,
};
use crate::room_terminal::{OpenRoomTerminal, TerminalActor, MAX_WAIT};
use crate::runtime::command::{KernelCaller, KernelCallerKind};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

const OPERATION: &str = "terminal.room";

impl KernelRuntimeState {
    /// Terminals belong to the Room's owner (users and the owner's agents).
    /// Other people in the Room get no access. The terminal runs on the machine
    /// of the agent it is for: this kernel, or the worker the agent is leased to.
    pub(crate) async fn room_terminal_for_client(
        &self,
        caller: &KernelCaller,
        request: RoomTerminalRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = self.authorize_room_terminal(caller, &request)?;
        let host = request
            .host_agent
            .as_deref()
            .or(request.acting_agent.as_deref());
        let everywhere = host.is_none() && matches!(request.operation, RoomTerminalOperation::List);
        let result = self
            .route_room_terminal(
                &request.session_id,
                actor,
                host,
                everywhere,
                request.operation,
            )
            .await?;
        Ok(LocalDaemonResponse::RoomTerminal { result })
    }

    /// Run `operation` where `host` (a Room agent) runs: on the worker it is
    /// leased to, else on this kernel. `everywhere` makes a list also cover the
    /// workers of the Room's agents. Clients and agent tools share this path.
    pub(crate) async fn route_room_terminal(
        &self,
        session_id: &str,
        actor: TerminalActor,
        host: Option<&str>,
        everywhere: bool,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let remote = match host {
            Some(agent_id) => self
                .owned
                .agent_store
                .get_agent(agent_id)?
                .remote_execution()
                .cloned(),
            None => None,
        };
        match (remote, host) {
            (Some(binding), Some(agent_id)) => Ok(self
                .forward_room_terminal(binding, actor, operation)
                .await?
                .hosted_by(agent_id)),
            _ => {
                let working_directory = self.room_project_directory(session_id)?;
                let mut result = self
                    .run_room_terminal_operation(
                        session_id,
                        working_directory,
                        actor.clone(),
                        operation,
                    )
                    .await?;
                if everywhere {
                    self.add_worker_terminals(session_id, actor, &mut result)
                        .await?;
                }
                Ok(result)
            }
        }
    }

    /// A Room-wide list also shows the terminals running on the workers its
    /// agents are leased to. A worker that cannot be reached fails the list: a
    /// missing tab must not look like a closed one.
    async fn add_worker_terminals(
        &self,
        session_id: &str,
        actor: TerminalActor,
        listed: &mut RoomTerminalResult,
    ) -> Result<(), DaemonError> {
        let RoomTerminalResult::Listed { terminals } = listed else {
            return Ok(());
        };
        let mut seen = std::collections::BTreeSet::new();
        for agent in self.session_agents(session_id) {
            let Some(binding) = agent.remote_execution().cloned() else {
                continue;
            };
            let worker = binding.worker_kernel_id.clone();
            let RoomTerminalResult::Listed { terminals: remote } = self
                .forward_room_terminal(binding, actor.clone(), RoomTerminalOperation::List)
                .await?
                .hosted_by(agent.id())
            else {
                continue;
            };
            terminals.extend(
                remote
                    .into_iter()
                    .filter(|terminal| seen.insert((worker.clone(), terminal.terminal_id.clone()))),
            );
        }
        Ok(())
    }

    /// Run one operation on this kernel's terminal service.
    pub(crate) async fn run_room_terminal_operation(
        &self,
        session_id: &str,
        working_directory: PathBuf,
        actor: TerminalActor,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let service = &self.owned.room_terminals;
        Ok(match operation {
            operation if operation.is_exec() => {
                self.run_room_exec_operation(session_id, &working_directory, operation)
                    .await?
            }
            RoomTerminalOperation::Open {
                cols,
                rows,
                title,
                command,
            } => {
                self.ensure_room_running(session_id)?;
                let terminal = service.open(OpenRoomTerminal {
                    session_id: session_id.to_string(),
                    working_directory,
                    cols,
                    rows,
                    title,
                    opened_by: actor,
                    command,
                })?;
                // A Room that ended while this opened has already swept its
                // terminals: ending is recorded before the sweep, so an open
                // that raced it sees the end here and removes its own shell.
                if let Err(ended) = self.ensure_room_running(session_id) {
                    service.end_session(session_id);
                    return Err(ended);
                }
                RoomTerminalResult::Opened { terminal }
            }
            RoomTerminalOperation::List => RoomTerminalResult::Listed {
                terminals: service.list(session_id),
            },
            RoomTerminalOperation::Snapshot { terminal_id } => RoomTerminalResult::Snapshot {
                snapshot: service.snapshot(session_id, &terminal_id)?,
            },
            RoomTerminalOperation::Input {
                terminal_id,
                data_base64,
            } => {
                let input = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .map_err(|_| refused("terminal input is not valid base64".to_string()))?;
                RoomTerminalResult::Input {
                    input: service
                        .input(session_id, &terminal_id, &actor, &input)
                        .await?,
                }
            }
            RoomTerminalOperation::Resize {
                terminal_id,
                cols,
                rows,
            } => RoomTerminalResult::Resized {
                terminal: service.resize(session_id, &terminal_id, &actor, cols, rows)?,
            },
            RoomTerminalOperation::ReadOutput {
                terminal_id,
                after_offset,
                wait_ms,
            } => RoomTerminalResult::Output {
                output: service
                    .read_output(
                        session_id,
                        &terminal_id,
                        after_offset,
                        Duration::from_millis(wait_ms),
                    )
                    .await?,
            },
            RoomTerminalOperation::Wait {
                terminal_id,
                watches,
                after_offset,
                timeout_ms,
            } => RoomTerminalResult::Waited {
                waited: service
                    .wait(
                        session_id,
                        &terminal_id,
                        &actor.to_string(),
                        watches,
                        after_offset,
                        Duration::from_millis(timeout_ms),
                    )
                    .await?,
            },
            RoomTerminalOperation::Close { terminal_id } => {
                service.close(session_id, &terminal_id, &actor)?;
                RoomTerminalResult::Closed
            }
            _ => unreachable!("exec operations handled above"),
        })
    }

    /// Ask the worker an agent is leased to for the same operation. The worker
    /// admits only the home kernel that holds the lease.
    async fn forward_room_terminal(
        &self,
        binding: crate::agent::RemoteAgentBinding,
        actor: TerminalActor,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        if !binding.relay_peer_protocol_compatible() {
            return Err(refused(format!(
                "worker kernel `{}` is too old for Room terminals; update it",
                binding.worker_kernel_id
            )));
        }
        let mut relay_config = self.owned.config_projection.snapshot();
        if let (Some(relay_url), Some(relay_token)) =
            (binding.relay_url.clone(), binding.relay_token.clone())
        {
            relay_config.apply_remote_relay_override(relay_url, relay_token);
        }
        let blocking = match &operation {
            RoomTerminalOperation::Wait { timeout_ms, .. } => {
                (*timeout_ms).min(MAX_WAIT.as_millis() as u64)
            }
            RoomTerminalOperation::ReadOutput { wait_ms, .. }
            | RoomTerminalOperation::ExecRead { wait_ms, .. } => {
                (*wait_ms).min(MAX_WAIT.as_millis() as u64)
            }
            _ => 0,
        };
        let timeout = Duration::from_millis(relay_config.relay_request_timeout_ms)
            .max(Duration::from_millis(blocking) + Duration::from_secs(20));
        let worker_kernel_id = binding.worker_kernel_id.clone();
        let response =
            crate::transport::relay_client::send_peer_request_via_temporary_connection_authorized(
                &relay_config,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(binding.worker_kernel_id),
                    daemon_alias: None,
                },
                RelayPeerRequest::RoomTerminal {
                    leased_agent_id: binding.leased_agent_id,
                    actor,
                    operation,
                },
                timeout,
                || self.authorize_current_external_command(),
            )
            .await
            .map_err(|error| worker_unreachable(&worker_kernel_id, error))?;
        match response {
            RelayPeerResponse::RoomTerminal { result } => Ok(result),
            other => Err(refused(format!(
                "unexpected worker response to a terminal request: {other:?}"
            ))),
        }
    }

    /// A terminal request from the home kernel that holds `leased_agent_id`'s
    /// lease: it runs in the agent's backing Room on this worker.
    pub(crate) async fn relay_room_terminal(
        &self,
        leased_agent_id: &str,
        actor: TerminalActor,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let id = leased_agent_id.to_string();
        let (backing_session_id, working_directory) = self
            .with_app_side_effect(move |app| {
                let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&id)?;
                runtime.leased_terminal_target(&id)
            })
            .await?;
        self.run_room_terminal_operation(&backing_session_id, working_directory, actor, operation)
            .await
    }

    pub(crate) fn end_room_terminals(&self, session_id: &str) {
        self.owned.room_execs.end_session(session_id);
        self.owned.room_terminals.end_session(session_id);
    }

    fn authorize_room_terminal(
        &self,
        caller: &KernelCaller,
        request: &RoomTerminalRequest,
    ) -> Result<TerminalActor, DaemonError> {
        if !matches!(
            caller.caller_kind,
            KernelCallerKind::LocalClient | KernelCallerKind::RemoteClient
        ) {
            return Err(refused(
                "only the Room owner's clients can use its terminals".to_string(),
            ));
        }
        let user_id = caller
            .user_id
            .as_deref()
            .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
        let session = self.owned.session_store.get_session(&request.session_id)?;
        // The enrolled terminal still owns the OS-local Rooms made before
        // enrollment; it keeps its authenticated identity as the actor.
        let owns_room = session.owner_user_id() == user_id
            || (caller.is_terminal_caller()
                && self.is_legacy_local_terminal_owner(&request.session_id, user_id));
        if !owns_room {
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: session.owner_user_id().to_string(),
                resource: request.session_id.clone(),
                operation: OPERATION,
            });
        }
        // The worker trusts the home kernel's lease, not the end user, so the
        // agent whose placement is used must belong to this Room too.
        if let Some(host_agent) = &request.host_agent {
            self.ensure_room_agent(&request.session_id, host_agent)?;
        }
        match &request.acting_agent {
            None => Ok(TerminalActor::User),
            Some(agent_id) => {
                self.ensure_room_agent(&request.session_id, agent_id)?;
                Ok(TerminalActor::Agent(agent_id.clone()))
            }
        }
    }

    /// Terminals belong to the Room's owner, and so do the agents that act for
    /// them: an agent made by another member of the Room never reaches them.
    pub(crate) fn ensure_room_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_id)
            .ok()
            .filter(|agent| agent.session_id() == session_id)
            .ok_or_else(|| refused(format!("this Room has no agent `{agent_id}`")))?;
        let session = self.owned.session_store.get_session(session_id)?;
        if agent.owner_user_id() != session.owner_user_id() {
            return Err(refused(format!(
                "agent `{agent_id}` does not act for the Room owner; only the owner's agents use the Room's terminals"
            )));
        }
        Ok(())
    }

    pub(super) fn ensure_room_running(&self, session_id: &str) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        if session.status() == crate::session::SessionStatus::Ended {
            return Err(refused(format!(
                "Room `{session_id}` has ended; reopen it before opening a terminal"
            )));
        }
        Ok(())
    }

    /// Where the Room's agents run their commands on this machine.
    pub(crate) fn room_project_directory(&self, session_id: &str) -> Result<PathBuf, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(PathBuf::from(session.worktree_id()))
    }
}

fn refused(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message,
    }
}

/// A worker that cannot be reached, in one sentence an outside agent can act on. The
/// transport's detail stays in the kernel log; the worker's own refusals pass through as
/// they came.
fn worker_unreachable(worker_kernel_id: &str, error: DaemonError) -> DaemonError {
    if matches!(error, DaemonError::MachineRequest { .. }) {
        return error;
    }
    tracing::warn!(%error, worker_kernel_id, "Room terminal worker could not be reached");
    crate::room_terminal::refusal(
        "the machine that runs this agent could not be reached, so its terminals are unavailable; retry, and if it keeps failing ask the user to check that machine"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreachable_worker_is_one_sentence_without_the_transport_detail() {
        let failure = worker_unreachable(
            "slice:m:1",
            DaemonError::LocalTransport {
                operation: "send relay peer request",
                message: "get_live_kernel: kernel slice:m:1 is not currently visible on relay"
                    .to_string(),
            },
        );
        let DaemonError::MachineRequest { code, message } = failure else {
            panic!("a public terminal refusal");
        };
        assert!(crate::room_terminal::is_public_code(&code));
        assert!(message.contains("could not be reached"));
        assert!(!message.contains("slice:m:1") && !message.contains("relay"));
    }

    #[test]
    fn a_worker_refusal_passes_through_as_it_came() {
        let refusal = crate::room_terminal::refusal(
            "terminal `t1` has exited; open a new terminal".to_string(),
        );
        let DaemonError::MachineRequest { message, .. } = worker_unreachable("slice:m:1", refusal)
        else {
            panic!("still a refusal");
        };
        assert_eq!(message, "terminal `t1` has exited; open a new terminal");
    }
}
