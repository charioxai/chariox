//! Caller-bound notification tools use the same trigger path as the shell.
use super::*;
use crate::local::{LocalDaemonResponse, WatcherRequest};
use crate::session::AgentTriggerNotificationSource;
use crate::transport::runtime_tools::{
    RuntimeToolResult, NOTIFICATION_SOURCES, NOTIFICATION_SUBSCRIBE, NOTIFICATION_UNSUBSCRIBE,
    NOTIFY_PERSON,
    WATCHER_ACTIVATE, WATCHER_CREATE, WATCHER_DRY_RUN, WATCHER_REMOVE, WATCHER_STATUS,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Subscribe {
    source: AgentTriggerNotificationSource,
    /// What the agent was asked and what it does when woken; shown as why the subscription exists.
    prompt: String,
    #[serde(default)]
    options: Vec<crate::local::NotificationOptionValue>,
    #[serde(default)]
    once: bool,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Unsubscribe {
    trigger_id: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NotifyPerson {
    title: String,
    body: Option<String>,
    link: Option<String>,
    #[serde(default)]
    priority: crate::local::PersonInboxPriority,
struct WatcherId {
    watcher_id: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Activate {
    watcher_id: String,
    paused: Option<bool>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    watcher_id: Option<String>,
}
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
        if name.starts_with("chariox.watcher.") {
            let payload = self
                .dispatch_watcher_tool(session, agent, name, arguments)
                .await?;
            self.authorize_current_external_response()?;
            return Ok(RuntimeToolResult { ok: true, payload });
        }
        let response = match name {
            NOTIFY_PERSON => {
                let args: NotifyPerson = serde_json::from_value(arguments).map_err(invalid)?;
                let card_id = crate::runtime::person_inbox::notify_person(
                    self,
                    session,
                    agent.id(),
                    agent.owner_user_id(),
                    crate::runtime::person_inbox::PersonNotice {
                        title: args.title,
                        body: args.body,
                        link: args.link,
                        priority: args.priority,
                    },
                )?;
                self.authorize_current_external_response()?;
                return Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({ "card_id": card_id }),
                });
            }
            NOTIFICATION_SOURCES => {
                let _: Empty = serde_json::from_value(arguments).map_err(invalid)?;
                self.owned
                    .ensure_owner_action_notification_source(session)?;
                let guest = self.notification_guest(session.owner_user_id(), agent.owner_user_id());
                if !guest {
                    let (stored, _, _) = self
                        .owned
                        .durable_state_store
                        .notification_inventory(session.owner_user_id())?;
                    self.owned
                        .sync_app_event_notification_sources(session.owner_user_id(), &stored)?;
                }
                let (sources, _, _) = self
                    .owned
                    .durable_state_store
                    .notification_inventory(session.owner_user_id())?;
                // The owner's agent also follows the owner's App events, whose source is the
                // installation rather than a Room.
                let guest = self.notification_guest(session.owner_user_id(), agent.owner_user_id());
                // A watcher is the owner's, wherever it was made: every Room of the owner lists it.
                let sources = sources
                    .into_iter()
                    .filter(|s| {
                        s.enabled
                            && (s.session_id == session.id()
                                || (!guest
                                    && s.kind == crate::local::NotificationSourceKind::AppEvent))
                                    && s.kind == crate::local::NotificationSourceKind::Watcher))
                    })
                    .collect::<Vec<_>>();
                let mut workflows = super::super::workflow_notification_router::summaries(
                    &self.owned.session_store.read(),
                    &sources,
                );
                // The owner's agent sees the owner's sources of the other kernels, as the owner's
                // list does; a collaborator's agent sees this Room's only.
                if !guest {
                    self.refresh_peer_notification_sources(session.owner_user_id())
                        .await?;
                    workflows.extend(
                        self.owned
                            .durable_state_store
                            .notification_cached_sources(session.owner_user_id())?,
                    );
                }
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
                    payload: serde_json::json!({"workflows":workflows.into_iter().map(|source| serde_json::json!({"source":{"workflow":{"source_id":source.source_id}},"display_name":source.display_name,"kernel_id":source.kernel_id,"session_id":source.session_id,"available":source.available,"options":source.options})).collect::<Vec<_>>(),"apps":apps}),
                });
            }
            NOTIFICATION_SUBSCRIBE => {
                let args: Subscribe = serde_json::from_value(arguments).map_err(invalid)?;
                if args.prompt.trim().is_empty() {
                    return Err(invalid(
                        "write in prompt what you were asked and what to do when the notification arrives",
                    ));
                }
                self.create_agent_trigger(
                    crate::local::CreateAgentTriggerRequest {
                        session_id: session.id().into(),
                        agent_id: agent.id().into(),
                        source: crate::local::AgentTriggerSourceRequest::Notification {
                            source: args.source,
                            options: args.options,
                            once: args.once,
                        },
                        busy_policy: None,
                        prompt: Some(args.prompt),
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
        let trigger = match response {
            crate::local::LocalDaemonResponse::AgentTriggerCreated { trigger, .. }
            | crate::local::LocalDaemonResponse::AgentTriggerDeleted { trigger, .. } => trigger,
            _ => return Err(invalid("unexpected notification subscription result")),
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"trigger_id":trigger.id(),"source":trigger.source(),"enabled":trigger.enabled()}),
        })
    }

    /// The watcher tools act for the Room's owner; the calling agent is responsible for what it makes.
    async fn dispatch_watcher_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        if self.notification_guest(session.owner_user_id(), agent.owner_user_id()) {
            return Err(invalid(
                "watchers are the Room owner's sources; a collaborator's agent cannot use them",
            ));
        }
        let caller = agent.owner_user_id();
        let request = match name {
            WATCHER_CREATE => {
                let (watcher_id, manifest, script) =
                    super::super::watchers::manifest_and_script(arguments)?;
                match watcher_id {
                    Some(watcher_id) => WatcherRequest::Update {
                        watcher_id,
                        manifest,
                        script,
                    },
                    None => WatcherRequest::Create {
                        session_id: session.id().into(),
                        manifest,
                        script,
                    },
                }
            }
            WATCHER_DRY_RUN => {
                let args: WatcherId = serde_json::from_value(arguments).map_err(invalid)?;
                WatcherRequest::DryRun {
                    watcher_id: args.watcher_id,
                }
            }
            WATCHER_ACTIVATE => {
                let args: Activate = serde_json::from_value(arguments).map_err(invalid)?;
                match args.paused {
                    None => WatcherRequest::Activate {
                        watcher_id: args.watcher_id,
                    },
                    Some(paused) => WatcherRequest::Pause {
                        watcher_id: args.watcher_id,
                        paused,
                    },
                }
            }
            WATCHER_STATUS => {
                let args: Status = serde_json::from_value(arguments).map_err(invalid)?;
                let Some(watcher_id) = args.watcher_id else {
                    let watchers = self
                        .owned
                        .durable_state_store
                        .watchers(Some(session.owner_user_id()))?;
                    return Ok(serde_json::json!({
                        "watchers": watchers.iter().map(|record| record.summary()).collect::<Vec<_>>(),
                    }));
                };
                let record = self.caller_watcher(&watcher_id, caller)?;
                return Ok(super::super::watchers::watcher_status(&record));
            }
            WATCHER_REMOVE => {
                let args: WatcherId = serde_json::from_value(arguments).map_err(invalid)?;
                WatcherRequest::Remove {
                    watcher_id: args.watcher_id,
                }
            }
            _ => return Err(invalid("unknown watcher tool")),
        };
        Ok(
            match self
                .watcher_request(request, caller, Some(agent.id()))
                .await?
            {
                LocalDaemonResponse::Watcher { watcher } => {
                    serde_json::json!({ "watcher": watcher })
                }
                LocalDaemonResponse::WatcherDryRun { result } => {
                    serde_json::to_value(result).map_err(|e| invalid(e.to_string()))?
                }
                LocalDaemonResponse::WatcherRemoved { watcher_id } => {
                    serde_json::json!({ "removed": watcher_id })
                }
                _ => return Err(invalid("unexpected watcher result")),
            },
        )
    }
}
