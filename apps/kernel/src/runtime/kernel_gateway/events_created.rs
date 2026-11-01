//! MP-08 / MP-09 / MP-10 / MP-11: follow the work an outside grant creates.
use super::*;
use crate::local::*;

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
        self.runtime_state
            .subscribe_outside_part(&origin.grant_id, true, targets)?;
        self.start_event_parts(origin);
        Ok(())
    }
}

impl KernelGateway {
    pub(crate) async fn follow_forwarded(
        self: &Arc<Self>,
        origin: &Origin,
        target: &str,
        request: &LocalDaemonRequest,
        response: &mut serde_json::Value,
    ) {
        if !follows_created(request) {
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
        *issue = Some(EventKernelIssue {
            code: "events_subscription_failed".into(),
            message: "Work was created, but automatic event subscription failed. Keep the returned IDs. Run chariox events subscription; if it ended, request a new access grant, otherwise remove unneeded targets or restore the connection, then subscribe using those IDs.".into(),
        });
    }
}
