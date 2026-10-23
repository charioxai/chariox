//! MP-08 / MP-09 / MP-10 / MP-11: follow the work an outside grant creates.
use super::*;
use crate::durable_state::agent_lifecycle::outside::Mutation;
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
            LocalDaemonResponse::AgentSpawned { agent } => vec![EventTarget {
                kernel: kernel.into(),
                session_id: agent.session_id().into(),
                agent_id: Some(agent.id().into()),
            }],
            LocalDaemonResponse::AgentsSpawned { agents } => agents
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
            .outside_store()
            .outside_mutate(Mutation::Subscribe {
                id: origin.grant_id.clone(),
                home: true,
                targets,
            })?;
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
        response: &serde_json::Value,
    ) -> Result<(), GatewayError> {
        if !follows_created(request) {
            return Ok(());
        }
        let kernel = self.resolve(target).await?.kernel_id;
        let response: LocalDaemonResponse =
            serde_json::from_value(response.clone()).map_err(|_| GatewayError::Remote {
                name: target.into(),
                code: "events_response_invalid".into(),
                message: "created work returned an invalid response".into(),
                retryable: false,
            })?;
        self.follow_created(origin,&kernel,request,&response).map_err(|_| GatewayError::Remote { name:target.into(),code:"events_subscription_failed".into(),message:"work was created but could not be followed; subscribe to the created work before continuing".into(),retryable:false })
    }
}
