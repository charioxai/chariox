//! Setup on another kernel of the same user: this kernel starts, watches and stops a Setup run that
//! executes on the target kernel, in the checkout that kernel holds for the Project. The run, its
//! output and its Stop are the target's own; this kernel only relays them over the same-owner channel.
use super::*;
use crate::local::{
    EnvironmentOperationRequest, EnvironmentOperationView, RemoteEnvironmentOperationRequest,
    RunProjectEnvironmentSetupRequest, RunRemoteProjectEnvironmentSetupRequest,
};
use crate::project_environment::environment_error;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

const PEER_TIMEOUT: Duration = Duration::from_secs(30);

impl KernelRuntimeState {
    async fn ask_setup_target(
        &self,
        owner: &str,
        kernel: &str,
        request: RelayPeerRequest,
    ) -> Result<EnvironmentOperationView, DaemonError> {
        match self
            .send_same_owner_peer_request(owner, kernel, request, PEER_TIMEOUT)
            .await
        {
            Ok(RelayPeerResponse::ProjectEnvironmentOperation { operation }) => Ok(operation),
            Ok(RelayPeerResponse::ProjectEnvironmentRefused { reason }) => {
                Err(environment_error(reason))
            }
            Ok(_) => Err(environment_error(
                "the machine answered with something unexpected",
            )),
            Err(_) => Err(environment_error("the machine is offline")),
        }
    }

    pub(crate) async fn run_remote_project_environment_setup(
        &self,
        request: RunRemoteProjectEnvironmentSetupRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.owned_project(&request.project_id, caller_user_id)?;
        let operation = self
            .ask_setup_target(
                caller_user_id,
                &request.target_kernel_id,
                RelayPeerRequest::RunProjectEnvironmentSetup {
                    project_id: request.project_id,
                    operation_id: request.operation_id,
                    checkout_path: request.checkout_path,
                },
            )
            .await?;
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }

    /// Get or Stop of an operation that runs on another kernel.
    pub(crate) async fn remote_environment_operation(
        &self,
        request: RemoteEnvironmentOperationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let peer = if request.stop {
            RelayPeerRequest::CancelProjectEnvironmentOperation {
                operation_id: request.operation_id,
            }
        } else {
            RelayPeerRequest::GetProjectEnvironmentOperation {
                operation_id: request.operation_id,
            }
        };
        let operation = self
            .ask_setup_target(caller_user_id, &request.target_kernel_id, peer)
            .await?;
        self.owned_project(&operation.project_id, caller_user_id)?;
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }

    /// The target side: the same Setup run, Get and Stop the owner's client would send here.
    pub(crate) fn receive_project_environment_setup_peer(
        &self,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let answered = match request {
            RelayPeerRequest::RunProjectEnvironmentSetup {
                project_id,
                operation_id,
                checkout_path,
            } => self.run_project_environment_setup(
                RunProjectEnvironmentSetupRequest {
                    project_id,
                    operation_id,
                    checkout_path,
                },
                owner,
            ),
            RelayPeerRequest::GetProjectEnvironmentOperation { operation_id } => self
                .read_environment_operation(&operation_id, owner)
                .map(|operation| LocalDaemonResponse::EnvironmentOperation { operation }),
            RelayPeerRequest::CancelProjectEnvironmentOperation { operation_id } => self
                .cancel_environment_operation(EnvironmentOperationRequest { operation_id }, owner),
            _ => return Err(environment_error("not a Setup request")),
        };
        Ok(match answered {
            Ok(LocalDaemonResponse::EnvironmentOperation { operation }) => {
                RelayPeerResponse::ProjectEnvironmentOperation { operation }
            }
            Ok(_) => RelayPeerResponse::ProjectEnvironmentRefused {
                reason: "the machine answered with something unexpected".into(),
            },
            Err(error) => RelayPeerResponse::ProjectEnvironmentRefused {
                reason: error.to_string(),
            },
        })
    }
}
