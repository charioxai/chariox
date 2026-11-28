//! Authorize a command on the kernel that owns its pipes and resolve its execution directory.
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::local::{
    CommandTarget, LocalDaemonResponse, RunCommandOperation as Operation, RunCommandRequest,
    RunCommandResult as ResultData,
};
use crate::room_exec::{error, KERNEL_SCOPE};
use crate::room_terminal::TerminalActor;
use crate::runtime::command::{KernelCaller, KernelCallerKind};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

impl KernelRuntimeState {
    pub(crate) fn authorize_queued_exec(
        &self,
        intent: &crate::room_exec::LaunchIntent,
    ) -> Result<(), DaemonError> {
        self.authorize_current_external_command()?;
        self.authorize_gateway_requester()?;
        if intent.session_id == KERNEL_SCOPE {
            return Ok(());
        }
        self.ensure_room_running(&intent.session_id)?;
        let session = self.owned.session_store.get_session(&intent.session_id)?;
        if session.owner_user_id() != intent.owner_user_id {
            return Err(error(
                "exec_owner_changed",
                "Room ownership changed before the command started",
            ));
        }
        Ok(())
    }

    pub(super) fn authorize_room_command(
        &self,
        caller: &KernelCaller,
        request: &RunCommandRequest,
    ) -> Result<TerminalActor, DaemonError> {
        let session_id = request.target.session_id().ok_or_else(|| {
            error(
                "exec_invalid_target",
                "this operation requires a Room workspace target",
            )
        })?;
        let acting_agent = request.acting_agent.as_deref();
        self.authorize_room_operation(
            caller,
            session_id,
            request.target.host_agent(),
            acting_agent,
            acting_agent.is_some_and(|id| self.names_origin_agent(id)),
            "command.run",
        )
    }

    pub(crate) async fn run_command_for_client(
        &self,
        caller: &KernelCaller,
        request: RunCommandRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        self.authorize_gateway_requester()?;
        let result = match &request.target {
            CommandTarget::Kernel => {
                let owner = caller
                    .user_id
                    .as_deref()
                    .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
                if !matches!(
                    caller.caller_kind,
                    KernelCallerKind::LocalClient | KernelCallerKind::RemoteClient
                ) || !self.is_gateway_owner(owner)
                    || request
                        .acting_agent
                        .as_deref()
                        .is_some_and(|id| !self.names_origin_agent(id))
                {
                    return Err(error("exec_access_denied", "running commands on this kernel requires the owner or a grant covering the kernel"));
                }
                let actor = request
                    .acting_agent
                    .map_or(TerminalActor::User, TerminalActor::Agent);
                self.run_command_operation(
                    KERNEL_SCOPE,
                    KERNEL_SCOPE,
                    owner,
                    None,
                    actor,
                    request.operation,
                )
                .await?
            }
            CommandTarget::RoomWorkspace { session_id } => {
                let actor = self.authorize_room_command(caller, &request)?;
                let session = self.owned.session_store.get_session(session_id)?;
                let directory = Path::new(session.worktree_id());
                self.run_command_operation(
                    session_id,
                    session_id,
                    session.owner_user_id(),
                    Some(directory),
                    actor,
                    request.operation,
                )
                .await?
            }
            CommandTarget::AgentSlice {
                session_id,
                agent_id,
            } => {
                let actor = self.authorize_room_command(caller, &request)?;
                let agent = self.owned.agent_store.get_agent(agent_id)?;
                if let Some(binding) = agent.remote_execution() {
                    self.forward_agent_command(binding, actor, request.operation)
                        .await?
                } else {
                    let session = self.owned.session_store.get_session(session_id)?;
                    let directory =
                        Path::new(agent.worktree_id().unwrap_or_else(|| session.worktree_id()));
                    let scope = agent_scope(session_id, agent_id);
                    self.run_command_operation(
                        session_id,
                        &scope,
                        session.owner_user_id(),
                        Some(directory),
                        actor,
                        request.operation,
                    )
                    .await?
                }
            }
        };
        Ok(LocalDaemonResponse::RunCommand { result })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_command_operation(
        &self,
        session_id: &str,
        scope: &str,
        owner: &str,
        root: Option<&Path>,
        actor: TerminalActor,
        operation: Operation,
    ) -> Result<ResultData, DaemonError> {
        let service = &self.owned.room_execs;
        let actor = match actor {
            TerminalActor::User => None,
            TerminalActor::Agent(id) => Some(id),
        };
        let opened = |exec_id| {
            if root.is_some() {
                if let Err(ended) = self.ensure_room_running(session_id) {
                    service.end_session(session_id);
                    return Err(ended);
                }
            }
            Ok(ResultData::ExecOpened { exec_id })
        };
        Ok(match operation {
            Operation::ExecList => ResultData::ExecListed {
                execs: service.list(scope),
            },
            Operation::ExecOpen {
                command,
                directory,
                timeout_ms,
            } => {
                if root.is_some() {
                    self.ensure_room_running(session_id)?;
                }
                if timeout_ms.is_some_and(|limit| limit == 0 || limit > 86_400_000) {
                    return Err(error(
                        "exec_invalid_timeout",
                        "command timeout must be between 1 ms and 24 hours",
                    ));
                }
                let cwd = command_directory(root, directory.as_deref())?;
                let id = service.open_command(
                    session_id,
                    scope,
                    owner,
                    &cwd,
                    &command,
                    self.clone(),
                    actor,
                    timeout_ms,
                )?;
                return opened(id);
            }
            Operation::ExecOpenJob { command, needs } => {
                let Some(root) = root.filter(|_| scope == session_id) else {
                    return Err(error(
                        "exec_invalid_target",
                        "a watched job requires a Room workspace",
                    ));
                };
                self.ensure_room_running(session_id)?;
                let id = service.open_job_as(
                    session_id,
                    owner,
                    root,
                    &command,
                    &needs,
                    Some(self.clone()),
                    actor,
                )?;
                return opened(id);
            }
            Operation::ExecStartNow { exec_id } => {
                service.start_now(scope, &exec_id)?;
                ResultData::ExecOpened { exec_id }
            }
            Operation::ExecPlace { .. } => {
                return Err(error(
                    "exec_place_not_here",
                    "place a held command on the kernel that holds it",
                ))
            }
            Operation::ExecWrite {
                exec_id,
                offset,
                data_base64,
            } => ResultData::ExecInput {
                next_offset: service.write(scope, &exec_id, offset, &data_base64)?,
            },
            Operation::ExecStdinClose { exec_id, offset } => ResultData::ExecInput {
                next_offset: service.close_stdin(scope, &exec_id, offset)?,
            },
            Operation::ExecRead {
                exec_id,
                after_cursor,
                wait_ms,
            } => ResultData::ExecOutput {
                output: service
                    .read(
                        scope,
                        &exec_id,
                        after_cursor,
                        Duration::from_millis(wait_ms),
                    )
                    .await?,
            },
            Operation::ExecCancel { exec_id } => {
                service.cancel(scope, &exec_id)?;
                ResultData::ExecCancelled
            }
            Operation::ExecClose { exec_id } => {
                service.close(scope, &exec_id)?;
                ResultData::ExecClosed
            }
        })
    }

    async fn forward_agent_command(
        &self,
        binding: &crate::agent::RemoteAgentBinding,
        actor: TerminalActor,
        operation: Operation,
    ) -> Result<ResultData, DaemonError> {
        if !binding.relay_peer_protocol_compatible() {
            return Err(error(
                "exec_worker_incompatible",
                "update the agent's kernel to use command execution",
            ));
        }
        let wait_ms = match &operation {
            Operation::ExecRead { wait_ms, .. } => *wait_ms,
            _ => 0,
        };
        let response = self
            .send_leased_worker_operation(
                binding,
                RelayPeerRequest::RunAgentCommand {
                    leased_agent_id: binding.leased_agent_id.clone(),
                    actor,
                    operation,
                },
                wait_ms,
            )
            .await?;
        match response {
            RelayPeerResponse::RunAgentCommand { result } => Ok(result),
            _ => Err(error(
                "exec_worker_response",
                "the agent's kernel returned an unexpected command response",
            )),
        }
    }

    pub(crate) async fn relay_agent_command(
        &self,
        leased_agent_id: &str,
        actor: TerminalActor,
        operation: Operation,
    ) -> Result<ResultData, DaemonError> {
        let id = leased_agent_id.to_string();
        let (session_id, directory) = self
            .with_app_side_effect(move |app| {
                let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&id)?;
                runtime.leased_terminal_target(&id)
            })
            .await?;
        let session = self.owned.session_store.get_session(&session_id)?;
        let scope = agent_scope(&session_id, leased_agent_id);
        self.run_command_operation(
            &session_id,
            &scope,
            session.owner_user_id(),
            Some(&directory),
            actor,
            operation,
        )
        .await
    }
}

fn agent_scope(session_id: &str, agent_id: &str) -> String {
    format!("agent:{session_id}:{agent_id}")
}

fn command_directory(root: Option<&Path>, directory: Option<&str>) -> Result<PathBuf, DaemonError> {
    let Some(root) = root else {
        return directory.map_or_else(crate::room_terminal::kernel_user_home, |directory| {
            let directory = PathBuf::from(directory);
            if directory.is_absolute() {
                Ok(directory)
            } else {
                Err(error(
                    "exec_invalid_directory",
                    "pass an absolute working directory on the selected kernel",
                ))
            }
        });
    };
    let Some(directory) = directory else {
        return Ok(root.to_path_buf());
    };
    let resolve = |path: &Path| {
        path.canonicalize().map_err(|failure| {
            error(
                "exec_invalid_directory",
                format!("cannot resolve command directory: {failure}"),
            )
        })
    };
    let root = resolve(root)?;
    let directory = resolve(&root.join(directory))?;
    if !directory.starts_with(&root) {
        return Err(error(
            "exec_outside_workspace",
            "choose a working directory inside the target workspace",
        ));
    }
    Ok(directory)
}
