use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, Weak};

use futures_util::{stream, StreamExt};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::time::{Duration, MissedTickBehavior};

use crate::error::DaemonError;
use crate::local::{
    ListEventConnectionDependenciesRequest, LocalDaemonRequest, LocalDaemonResponse,
    RemoveEventConnectionRequest, SetWorkflowEventBindingStatusRequest,
};
use crate::runtime::command::KernelCommand;
use crate::runtime::event_catalog_control::execute_event_catalog_request_with_client;
use crate::session::WorkflowEventBindingStatus;

use super::CommandRouter;

const AUTHORIZATION_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(2);
const AUTHORIZATION_RECONCILIATION_BATCH_SIZE: usize = 64;
const AUTHORIZATION_RECONCILIATION_CONCURRENCY: usize = 8;

type ConnectionLaneKey = (String, String);
type ConnectionLane = Weak<Mutex<()>>;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct EventConnectionReconciliationSummary {
    pub attempted: usize,
    pub observed: usize,
    pub completed: usize,
    pub failed: usize,
}

#[derive(Debug, Default)]
struct AuthorizationReconciliationOutcome {
    observed: bool,
    completed: bool,
    failed: bool,
}

#[derive(Clone, Default)]
pub(super) struct EventConnectionOperationLanes {
    lanes: Arc<StdMutex<HashMap<ConnectionLaneKey, ConnectionLane>>>,
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
    pub(crate) async fn reconcile_pending_event_connections(
        &self,
    ) -> Result<EventConnectionReconciliationSummary, DaemonError> {
        let authorizations = self
            .runtime_state
            .event_connection_registry()
            .reconcilable_authorizations()?;
        let authorizations = authorizations
            .into_iter()
            .take(AUTHORIZATION_RECONCILIATION_BATCH_SIZE)
            .collect::<Vec<_>>();
        let mut summary = EventConnectionReconciliationSummary {
            attempted: authorizations.len(),
            ..Default::default()
        };
        let outcomes = stream::iter(authorizations.into_iter().map(
            |(caller_user_id, authorization)| async move {
                self.reconcile_event_connection_authorization(caller_user_id, authorization)
                    .await
            },
        ))
        .buffer_unordered(AUTHORIZATION_RECONCILIATION_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        for outcome in outcomes {
            summary.observed += usize::from(outcome.observed);
            summary.completed += usize::from(outcome.completed);
            summary.failed += usize::from(outcome.failed);
        }
        Ok(summary)
    }

    async fn reconcile_event_connection_authorization(
        &self,
        caller_user_id: String,
        authorization: crate::local::EventConnectionAuthorization,
    ) -> AuthorizationReconciliationOutcome {
        let Some(connection_id) = authorization.connection_id.as_deref() else {
            return AuthorizationReconciliationOutcome {
                failed: true,
                ..Default::default()
            };
        };
        let _connection_guard = self
            .event_connection_lanes
            .lock(&caller_user_id, connection_id)
            .await;
        let result = execute_event_catalog_request_with_client(
            &self.runtime_state,
            &self.config_projection,
            &self.aegs_management_http_client,
            &caller_user_id,
            LocalDaemonRequest::ObserveEventConnectionAuthorization(
                crate::local::ObserveEventConnectionAuthorizationRequest {
                    authorization_id: authorization.authorization_id.clone(),
                },
            ),
        )
        .await;
        match result {
            Ok(LocalDaemonResponse::EventConnectionAuthorizationObserved {
                connection: Some(connection),
                ..
            }) => AuthorizationReconciliationOutcome {
                observed: true,
                completed: connection.status != crate::local::EventConnectionStatus::Pending,
                failed: false,
            },
            Ok(LocalDaemonResponse::EventConnectionAuthorizationObserved {
                connection: None,
                ..
            }) => AuthorizationReconciliationOutcome::default(),
            Ok(_) => AuthorizationReconciliationOutcome {
                failed: true,
                ..Default::default()
            },
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.event_connection_reconciliation",
                    "pending event connection authorization reconciliation failed",
                    serde_json::json!({
                        "generator_id": authorization.generator_id,
                        "authorization_id": authorization.authorization_id,
                        "error": error.to_string(),
                    }),
                );
                AuthorizationReconciliationOutcome {
                    failed: true,
                    ..Default::default()
                }
            }
        }
    }

    pub(crate) async fn run_event_connection_authorization_reconciler(
        &self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut reconciliation = tokio::time::interval(AUTHORIZATION_RECONCILIATION_INTERVAL);
        reconciliation.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            if *shutdown.borrow() {
                return;
            }
            tokio::select! {
                _ = reconciliation.tick() => {
                    let result = tokio::select! {
                        result = self.reconcile_pending_event_connections() => result,
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                return;
                            }
                            continue;
                        }
                    };
                    match result {
                        Ok(summary) if summary.completed != 0 => {
                            crate::logging::info_with_fields(
                                "daemon.event_connection_reconciliation",
                                "event connection authorization completed in background",
                                serde_json::json!({
                                    "completed_count": summary.completed,
                                    "attempted_count": summary.attempted,
                                }),
                            );
                        }
                        Ok(_) => {}
                        Err(error) => {
                            crate::logging::warn_with_fields(
                                "daemon.event_connection_reconciliation",
                                "event connection authorization registry reconciliation failed",
                                serde_json::json!({"error": error.to_string()}),
                            );
                        }
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
            }
        }
    }

    pub(super) async fn remove_event_connection(
        &self,
        command: &KernelCommand,
        caller_user_id: &str,
        request: RemoveEventConnectionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let dependencies = execute_event_catalog_request_with_client(
            &self.runtime_state,
            &self.config_projection,
            &self.aegs_management_http_client,
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

        execute_event_catalog_request_with_client(
            &self.runtime_state,
            &self.config_projection,
            &self.aegs_management_http_client,
            caller_user_id,
            LocalDaemonRequest::RemoveEventConnection(request),
        )
        .await
    }
}
