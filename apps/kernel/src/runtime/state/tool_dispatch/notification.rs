//! Caller-bound notification tools use the same trigger path as the shell.
use super::*;
use crate::session::AgentTriggerNotificationSource;
use crate::transport::runtime_tools::{
    RuntimeToolResult, NOTIFICATION_SOURCES, NOTIFICATION_SUBSCRIBE, NOTIFICATION_UNSUBSCRIBE,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Subscribe {
    source: AgentTriggerNotificationSource,
    prompt: Option<String>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Unsubscribe {
    trigger_id: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
fn invalid(error: impl std::fmt::Display) -> DaemonError {
    crate::durable_state::workflow_notifications::error(error.to_string())
}

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn dispatch_notification_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if !self.room_agent_tools_enabled()
            || agent.archived()
            || agent.session_id() != session.id()
            || session.status() == crate::session::SessionStatus::Ended
        {
            return Err(invalid("notification tools require a live Room agent"));
        }
        self.authorize_room_provider_epoch(
            self.room_provider_origin
                .as_ref()
                .map(|(agent, _)| agent.as_str()),
            self.room_provider_origin
                .as_ref()
                .map(|(_, run)| run.as_str()),
        )?;
        let response = match name {
            NOTIFICATION_SOURCES => {
                let _: Empty = serde_json::from_value(arguments).map_err(invalid)?;
                self.owned
                    .ensure_owner_action_notification_source(session)?;
                let (sources, _, _) = self
                    .owned
                    .durable_state_store
                    .notification_inventory(session.owner_user_id())?;
                let sources = sources
                    .into_iter()
                    .filter(|s| s.session_id == session.id() && s.enabled)
                    .collect::<Vec<_>>();
                let workflows = super::super::workflow_notification_router::summaries(
                    &self.owned.session_store.read(),
                    &sources,
                );
                let mut apps = Vec::new();
                for installation in self.room_notification_apps(session.id()) {
                    let catalog = self
                        .app_catalog(session.owner_user_id(), &installation)
                        .await
                        .map_err(|code| invalid(format!("App event unavailable: {code:?}")))?;
                    let installed = self
                        .owned
                        .durable_state_store
                        .get_app_installation(session.owner_user_id(), &installation)
                        .map_err(invalid)?;
                    for (event, label) in catalog.outgoing_events() {
                        apps.push(serde_json::json!({"source":{"app":{"installation_id":installation,"event_name":event}},"display_name":format!("{} · {}",installed.app_id,label.unwrap_or(event)),"kernel_id":self.owned.config_projection.snapshot().daemon_id,"session_id":session.id(),"available":true}));
                    }
                }
                self.authorize_current_external_response()?;
                return Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({"workflows":workflows.into_iter().map(|source| serde_json::json!({"source":{"workflow":{"source_id":source.source_id}},"display_name":source.display_name,"kernel_id":source.kernel_id,"session_id":source.session_id,"available":source.available})).collect::<Vec<_>>(),"apps":apps}),
                });
            }
            NOTIFICATION_SUBSCRIBE => {
                let args: Subscribe = serde_json::from_value(arguments).map_err(invalid)?;
                self.create_agent_trigger(
                    crate::local::CreateAgentTriggerRequest {
                        session_id: session.id().into(),
                        agent_id: agent.id().into(),
                        source: crate::local::AgentTriggerSourceRequest::Notification {
                            source: args.source,
                        },
                        busy_policy: None,
                        prompt: args.prompt,
                    },
                    agent.owner_user_id(),
                )
                .await?
            }
            NOTIFICATION_UNSUBSCRIBE => {
                let args: Unsubscribe = serde_json::from_value(arguments).map_err(invalid)?;
                if !session.agent_triggers().iter().any(|trigger| {
                    trigger.id() == args.trigger_id
                        && trigger.agent_id() == agent.id()
                        && matches!(
                            trigger.source(),
                            crate::session::AgentTriggerSource::Notification { .. }
                        )
                }) {
                    return Err(invalid(
                        "notification trigger does not belong to the calling agent",
                    ));
                }
                self.delete_agent_trigger(
                    crate::local::DeleteAgentTriggerRequest {
                        session_id: session.id().into(),
                        trigger_id: args.trigger_id,
                    },
                    agent.owner_user_id(),
                )
                .await?
            }
            _ => return Err(invalid("unknown notification tool")),
        };
        self.authorize_current_external_response()?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::to_value(response).map_err(invalid)?,
        })
    }
}
