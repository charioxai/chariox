use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, Weak};

use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::error::DaemonError;
use crate::local::{
    ListEventConnectionDependenciesRequest, LocalDaemonRequest, LocalDaemonResponse,
    RemoveEventConnectionRequest, SetWorkflowEventBindingStatusRequest,
};
use crate::runtime::command::KernelCommand;
use crate::runtime::event_catalog_control::execute_event_catalog_request;
use crate::session::WorkflowEventBindingStatus;

use super::CommandRouter;

#[derive(Clone, Default)]
pub(super) struct EventConnectionOperationLanes {
    lanes: Arc<StdMutex<HashMap<(String, String), Weak<Mutex<()>>>>>,
}

impl EventConnectionOperationLanes {
    pub(super) async fn lock(
        &self,
        caller_user_id: &str,
        connection_id: &str,
    ) -> OwnedMutexGuard<()> {
        let lane_key = (caller_user_id.to_string(), connection_id.to_string());
        let lane = {
            let mut lanes = self
                .lanes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(lane) = lanes.get(&lane_key).and_then(Weak::upgrade) {
                lane
            } else {
                let lane = Arc::new(Mutex::new(()));
                lanes.insert(lane_key, Arc::downgrade(&lane));
                lane
            }
        };
        lane.lock_owned().await
    }
}

impl CommandRouter {
    pub(super) async fn remove_event_connection(
        &self,
        command: &KernelCommand,
        caller_user_id: &str,
        request: RemoveEventConnectionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let dependencies = execute_event_catalog_request(
            &self.runtime_state,
            &self.config_projection,
            caller_user_id,
            LocalDaemonRequest::ListEventConnectionDependencies(
                ListEventConnectionDependenciesRequest {
                    connection_id: request.connection_id.clone(),
                },
            ),
        )
        .await?;
        let LocalDaemonResponse::EventConnectionDependencies { dependencies, .. } = dependencies
        else {
            return Err(DaemonError::LocalTransport {
                operation: "remove event connection",
                message: "kernel returned an unexpected dependency response".to_string(),
            });
        };

        if request.confirm {
            for dependency in dependencies
                .iter()
                .filter(|dependency| dependency.status != WorkflowEventBindingStatus::Tombstoned)
            {
                let deactivate_request = LocalDaemonRequest::SetWorkflowEventBindingStatus(
                    SetWorkflowEventBindingStatusRequest {
                        session_id: dependency.session_id.clone(),
                        binding_id: dependency.binding_id.clone(),
                        status: WorkflowEventBindingStatus::Tombstoned,
                    },
                );
                let deactivate_command = KernelCommand::from_local_request_with_caller(
                    format!(
                        "{}:event-connection-remove:{}",
                        command.command_id, dependency.binding_id
                    ),
                    command.source.clone(),
                    command.caller.clone(),
                    Some(command.correlation_id.clone()),
                    Some(command.command_id.clone()),
                    &deactivate_request,
                );
                self.workflow_runtime
                    .dispatch_workflow_command(deactivate_command, deactivate_request)
                    .await?;
            }
        }

        execute_event_catalog_request(
            &self.runtime_state,
            &self.config_projection,
            caller_user_id,
            LocalDaemonRequest::RemoveEventConnection(request),
        )
        .await
    }
}
