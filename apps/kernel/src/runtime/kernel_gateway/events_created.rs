//! MP-08 / MP-09 / MP-10 / MP-11: follow the work an outside grant creates.
use super::*;
use crate::local::*;
use serde_json::Value;

pub(super) fn follows_created(request: &LocalDaemonRequest) -> bool {
    match request {
        LocalDaemonRequest::CreateSession(request) => !request.no_subscribe,
        LocalDaemonRequest::SpawnAgent(request) => !request.no_subscribe,
        LocalDaemonRequest::SpawnAgents(request) => !request.no_subscribe,
        _ => false,
    }
}
impl KernelGateway {
    pub(crate) fn follow_created(
        self: &Arc<Self>,
        origin: &Origin,
        kernel: &str,
        request: &LocalDaemonRequest,
        response: &LocalDaemonResponse,
    ) -> Result<(), crate::error::DaemonError> {
        if !follows_created(request) {
            return Ok(());
        }
        let targets = match response {
            LocalDaemonResponse::SessionCreated { session, .. } => vec![EventTarget {
                kernel: kernel.into(),
                session_id: session.id().into(),
                agent_id: None,
            }],
            LocalDaemonResponse::AgentSpawned { agent, .. } => vec![EventTarget {
                kernel: kernel.into(),
                session_id: agent.session_id().into(),
                agent_id: Some(agent.id().into()),
            }],
            LocalDaemonResponse::AgentsSpawned { agents, .. } => agents
                .iter()
                .map(|agent| EventTarget {
                    kernel: kernel.into(),
                    session_id: agent.session_id().into(),
                    agent_id: Some(agent.id().into()),
                })
                .collect(),
            _ => return Ok(()),
        };
        self.subscribe_created_targets(origin, targets)
    }
}

impl KernelGateway {
    pub(crate) async fn follow_forwarded(
        self: &Arc<Self>,
        origin: &Origin,
        target: &str,
        request: &LocalDaemonRequest,
        response: &mut Value,
    ) {
        if !follows_created(request) {
            return;
        }
        if matches!(request, LocalDaemonRequest::CreateSession(_)) {
            let Some(session_id) = response
                .pointer("/SessionCreated/session_id")
                .and_then(Value::as_str)
            else {
                return;
            };
            let failed = match self.resolve(target).await {
                Ok(kernel) => self
                    .subscribe_created_targets(
                        origin,
                        vec![EventTarget {
                            kernel: kernel.kernel_id,
                            session_id: session_id.to_owned(),
                            agent_id: None,
                        }],
                    )
                    .is_err(),
                Err(_) => true,
            };
            if failed {
                response["SessionCreated"]["event_subscription_error"] =
                    serde_json::to_value(event_subscription_failure())
                        .expect("event issue serializes");
            }
            return;
        }
        let Ok(mut created) = serde_json::from_value::<LocalDaemonResponse>(response.clone())
        else {
            // Preserve the target's reply; decoding that reply belongs to the caller.
            return;
        };
        let failed = match self.resolve(target).await {
            Ok(kernel) => self
                .follow_created(origin, &kernel.kernel_id, request, &created)
                .is_err(),
            Err(_) => true,
        };
        if failed {
            created.record_event_subscription_failure();
            *response = serde_json::to_value(created).expect("created work response serializes");
        }
    }

    fn subscribe_created_targets(
        self: &Arc<Self>,
        origin: &Origin,
        targets: Vec<EventTarget>,
    ) -> Result<(), crate::error::DaemonError> {
        self.runtime_state
            .subscribe_outside_part(&origin.grant_id, true, targets)?;
        self.start_event_parts(origin);
        Ok(())
    }
}

impl LocalDaemonResponse {
    pub(crate) fn record_event_subscription_failure(&mut self) {
        let issue = match self {
            Self::SessionCreated {
                event_subscription_error,
                ..
            }
            | Self::AgentSpawned {
                event_subscription_error,
                ..
            }
            | Self::AgentsSpawned {
                event_subscription_error,
                ..
            } => event_subscription_error,
            _ => return,
        };
        *issue = Some(event_subscription_failure());
    }
}

fn event_subscription_failure() -> EventKernelIssue {
    EventKernelIssue {
        code: "events_subscription_failed".into(),
        message: "Work was created, but automatic event subscription failed. Keep the returned IDs. Run chariox events subscription; if it ended, request a new access grant, otherwise remove unneeded targets or restore the connection, then subscribe using those IDs.".into(),
    }
}
