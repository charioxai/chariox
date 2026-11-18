//! The Notification center's subscriptions table (docs/NOTIFICATION_CENTER_PLAN.md section 6):
//! every subscription whose subscriber lives on this kernel, in a Room the caller owns, with
//! what it picked, its counters and its health. Pause, change of subscriber, removal, the dry
//! run and the trace each act on one row of it. An agent's listener on an App event is a row
//! as well, keyed by its App automation, and every one of these acts on it the same way.
use super::agent_trigger_notification::{notification_subscription_id, RESPONSIBLE_LISTENER};
use super::workflow_notification_router::{source_available, summaries};
use super::*;
use crate::durable_state::workflow_notifications::{
    self as store, subscription_settings::SubscriptionSettings, NotificationOperation,
};
use crate::local::{
    AgentTriggerSourceRequest, CreateAgentTriggerRequest, DeleteAgentTriggerRequest,
    DetachWorkflowNotificationRequest, LocalDaemonResponse, NotificationSourceKind,
    NotificationSubscriptionChange, NotificationSubscriptionHealth, NotificationSubscriptionRow,
    NotificationTraceOutcome, PersonResponsibilityRow, UpdateNotificationSubscriptionRequest,
    WorkflowNotificationSourceSummary, WorkflowNotificationSubscription,
    WorkflowNotificationTargetKind,
};
use crate::session::{AgentTrigger, AgentTriggerNotificationSource, AgentTriggerSource};

impl KernelRuntimeState {
    pub(crate) fn list_notification_subscriptions(
        &self,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::NotificationSubscriptions {
            subscriptions: self.notification_subscription_rows(caller)?,
            responsibilities: self.person_responsibilities(caller),
        })
    }

    /// The workflows of the caller's Rooms that the person answers for: each failed or held
    /// run comes to the Inbox as a card, so the table shows "You" as the subscriber.
    fn person_responsibilities(&self, caller: &str) -> Vec<PersonResponsibilityRow> {
        self.owned
            .session_store
            .list_sessions()
            .iter()
            .filter(|room| self.acts_for_room_owner(room, caller))
            .flat_map(|room| {
                room.workflows()
                    .iter()
                    .filter(|workflow| workflow.responsible().is_person())
                    .map(|workflow| {
                        let needed = room
                            .workflow_runs()
                            .iter()
                            .filter(|run| run.workflow_id() == workflow.id())
                            .filter(|run| run.outcome().is_some())
                            .map(|run| run.completed_at_ms().unwrap_or(run.created_at_ms()))
                            .collect::<Vec<_>>();
                        PersonResponsibilityRow {
                            session_id: room.id().to_owned(),
                            workflow_id: workflow.id().to_owned(),
                            workflow_name: store::naming::workflow_name(workflow),
                            fired: needed.len() as u64,
                            last_fired_at_ms: needed.into_iter().max(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(crate) async fn update_notification_subscription(
        &self,
        request: UpdateNotificationSubscriptionRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let row = self.notification_subscription_row(&request.subscription_id, caller)?;
        match request.change {
            NotificationSubscriptionChange::Paused { paused } => {
                self.owned
                    .durable_state_store
                    .notify(NotificationOperation::Pause {
                        subscription_id: row.subscription.subscription_id,
                        paused,
                    })?;
            }
            NotificationSubscriptionChange::Subscriber { agent_id } => {
                self.change_notification_subscriber(row, agent_id, caller)
                    .await?;
            }
        }
        self.record_person_inbox_change();
        self.list_notification_subscriptions(caller)
    }

    pub(crate) async fn remove_notification_subscription(
        &self,
        subscription_id: &str,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let row = self.notification_subscription_row(subscription_id, caller)?;
        if row.responsible_for.is_some() {
            return Err(store::error(RESPONSIBLE_LISTENER));
        }
        self.end_notification_subscription(&row.subscription, caller)
            .await?;
        self.record_person_inbox_change();
        self.list_notification_subscriptions(caller)
    }

    /// A dry run against the latest occurrence this kernel holds: nothing is queued and no
    /// counter moves.
    pub(crate) fn test_notification_subscription(
        &self,
        subscription_id: &str,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let row = self.notification_subscription_row(subscription_id, caller)?;
        let subscriber = row.subscriber_name.as_deref().unwrap_or("its subscriber");
        let store = &self.owned.durable_state_store;
        let dry_run = match self.listened_source(&row.subscription) {
            Some(AgentTriggerNotificationSource::App {
                installation_id, ..
            }) => store.notification_app_dry_run(
                &row.subscription.owner_user_id,
                &installation_id,
                &row.subscription.subscription_id,
            )?,
            _ => store.notification_dry_run(&row.subscription)?,
        };
        let result = match dry_run {
            None => "Nothing has arrived from this source yet, so there is nothing to test. Nothing was sent.".to_owned(),
            Some(dry_run) => match dry_run.skipped {
                None => format!(
                    "\"{}\" would wake {subscriber}. Nothing was sent.",
                    dry_run.title
                ),
                Some((NotificationTraceOutcome::Filtered, detail)) => format!(
                    "\"{}\" would be filtered out: {}. Nothing was sent.",
                    dry_run.title,
                    detail.unwrap_or_default()
                ),
                Some((NotificationTraceOutcome::Paused, _)) => format!(
                    "\"{}\" would wait: the subscription is paused. Nothing was sent.",
                    dry_run.title
                ),
                Some((_, _)) => format!(
                    "\"{}\" would wait: the source no longer declares the picked options. Nothing was sent.",
                    dry_run.title
                ),
            },
        };
        Ok(LocalDaemonResponse::NotificationSubscriptionTested { result })
    }

    pub(crate) fn read_notification_trace(
        &self,
        subscription_id: &str,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let row = self.notification_subscription_row(subscription_id, caller)?;
        Ok(LocalDaemonResponse::NotificationTrace {
            lines: self
                .owned
                .durable_state_store
                .notification_trace(&row.subscription.subscription_id)?,
            subscription_id: row.subscription.subscription_id,
        })
    }

    fn notification_subscription_row(
        &self,
        subscription_id: &str,
        caller: &str,
    ) -> Result<NotificationSubscriptionRow, DaemonError> {
        self.notification_subscription_rows(caller)?
            .into_iter()
            .find(|row| row.subscription.subscription_id == subscription_id)
            .ok_or_else(|| {
                store::error(format!(
                    "No subscription {subscription_id} of yours on this kernel. List the subscriptions again and pick one from it."
                ))
            })
    }

    /// The subscriptions of agents and workflows in the Rooms the caller owns here.
    pub(crate) fn notification_subscription_rows(
        &self,
        caller: &str,
    ) -> Result<Vec<NotificationSubscriptionRow>, DaemonError> {
        let home = self.owned.config_projection.snapshot().daemon_id;
        let store = &self.owned.durable_state_store;
        let sessions = self.owned.session_store.read();
        let rooms: Vec<_> = sessions
            .list_sessions()
            .into_iter()
            .filter(|session| self.acts_for_room_owner(session, caller))
            .collect();
        let owners: BTreeSet<String> = rooms
            .iter()
            .map(|session| session.owner_user_id().to_owned())
            .collect();
        let mut rows = Vec::new();
        for owner in owners {
            let (mut local, subscriptions, _) = store.notification_inventory(&owner)?;
            for source in &mut local {
                source.available = source_available(&sessions, source);
            }
            let mut sources = summaries(&sessions, &local);
            sources.extend(store.notification_cached_sources(&owner)?);
            for subscription in subscriptions {
                let Some(room) = rooms.iter().find(|room| {
                    room.id() == subscription.session_id && subscription.target_kernel_id == home
                }) else {
                    continue;
                };
                // The session store's Room carries no agents; the agent store names them.
                let subscriber_name = match subscription.target_kind {
                    WorkflowNotificationTargetKind::Agent => self
                        .owned
                        .agent_notification_target_live(room, &subscription)
                        .then(|| self.owned.agent_store.get_agent(&subscription.endpoint_id))
                        .and_then(Result::ok)
                        .map(|agent| agent.alias().unwrap_or(agent.agent_ref()).to_owned()),
                    WorkflowNotificationTargetKind::WorkflowEndpoint => sessions
                        .resolve_workflow_ref(room.id(), &subscription.workflow_id)
                        .ok()
                        .map(|workflow| store::naming::workflow_name(&workflow)),
                    WorkflowNotificationTargetKind::OutsideAgent => continue,
                };
                let trigger = subscription.trigger_id.as_deref().and_then(|trigger_id| {
                    room.agent_triggers()
                        .iter()
                        .find(|trigger| trigger.id() == trigger_id)
                });
                let prompt = trigger.map(|trigger| trigger.prompt().to_owned());
                // A workflow of another kernel is named by its id; this kernel cannot read it.
                let responsible_for =
                    trigger
                        .and_then(|trigger| trigger.responsible_for())
                        .map(|workflow| {
                            sessions
                                .resolve_workflow_ref(&workflow.room_id, &workflow.workflow_id)
                                .map(|definition| store::naming::workflow_name(&definition))
                                .unwrap_or_else(|_| workflow.workflow_id.clone())
                        });
                let source = sources
                    .iter()
                    .find(|source| source.source_id == subscription.source_id && source.available)
                    .cloned();
                let settings = store.notification_subscription_settings(&subscription)?;
                let health = health(source.is_some(), subscriber_name.is_some(), &settings);
                rows.push(NotificationSubscriptionRow {
                    subscription,
                    source,
                    subscriber_name,
                    prompt,
                    options: settings.options,
                    once: settings.once,
                    paused: settings.paused,
                    fired: settings.fired,
                    last_fired_at_ms: settings.last_fired_at_ms,
                    loop_stopped_at_ms: settings.loop_stopped_at_ms,
                    responsible_for,
                    health,
                });
            }
            for room in rooms.iter().filter(|room| room.owner_user_id() == owner) {
                for trigger in room.agent_triggers() {
                    if let Some(row) = self.app_listener_row(room, trigger, &home)? {
                        rows.push(row);
                    }
                }
            }
        }
        Ok(rows)
    }

    /// An agent's listener on an App event: the row its App automation keys. The App's
    /// installation is the source; it is gone once the App is uninstalled.
    fn app_listener_row(
        &self,
        room: &crate::session::RuntimeSession,
        trigger: &AgentTrigger,
        home: &str,
    ) -> Result<Option<NotificationSubscriptionRow>, DaemonError> {
        let AgentTriggerSource::Notification {
            source:
                AgentTriggerNotificationSource::App {
                    installation_id,
                    event_name,
                },
            subscription_id,
            ..
        } = trigger.source()
        else {
            return Ok(None);
        };
        let owner = room.owner_user_id();
        let subscription = WorkflowNotificationSubscription {
            subscription_id: subscription_id.clone(),
            source_id: installation_id.clone(),
            owner_user_id: owner.to_owned(),
            created_by_user_id: trigger.created_by_user_id().unwrap_or(owner).to_owned(),
            target_kernel_id: home.to_owned(),
            target_kind: WorkflowNotificationTargetKind::Agent,
            session_id: room.id().to_owned(),
            workflow_id: String::new(),
            publication_id: trigger.id().to_owned(),
            endpoint_id: trigger.agent_id().to_owned(),
            queue_id: String::new(),
            source_available: true,
            source_kernel_id: home.to_owned(),
            agent_id: Some(trigger.agent_id().to_owned()),
            trigger_id: Some(trigger.id().to_owned()),
        };
        let source = self
            .owned
            .durable_state_store
            .get_app_installation(owner, installation_id)
            .ok()
            .map(|installed| WorkflowNotificationSourceSummary {
                kind: NotificationSourceKind::AppEvent,
                source_id: installation_id.clone(),
                kernel_id: home.to_owned(),
                session_id: room.id().to_owned(),
                workflow_id: String::new(),
                exit_node_id: event_name.clone(),
                exit_ordinal: 0,
                name: event_name.clone(),
                display_name: format!("{} · {event_name}", installed.app_id),
                available: true,
                options: Vec::new(),
            });
        let subscriber_name = self
            .owned
            .agent_store
            .get_agent(trigger.agent_id())
            .ok()
            .filter(|agent| agent.session_id() == room.id())
            .map(|agent| agent.alias().unwrap_or(agent.agent_ref()).to_owned());
        let settings = self
            .owned
            .durable_state_store
            .notification_subscription_settings(&subscription)?;
        let health = health(source.is_some(), subscriber_name.is_some(), &settings);
        Ok(Some(NotificationSubscriptionRow {
            subscription,
            source,
            subscriber_name,
            prompt: Some(trigger.prompt().to_owned()),
            options: settings.options,
            once: settings.once,
            paused: settings.paused,
            fired: settings.fired,
            last_fired_at_ms: settings.last_fired_at_ms,
            loop_stopped_at_ms: settings.loop_stopped_at_ms,
            responsible_for: None,
            health,
        }))
    }

    /// What an agent's subscription listens to, as its trigger says.
    fn listened_source(
        &self,
        subscription: &WorkflowNotificationSubscription,
    ) -> Option<AgentTriggerNotificationSource> {
        let trigger_id = subscription.trigger_id.as_deref()?;
        let room = self
            .owned
            .session_store
            .get_session(&subscription.session_id)
            .ok()?;
        room.agent_triggers()
            .iter()
            .find(|trigger| trigger.id() == trigger_id)
            .and_then(|trigger| match trigger.source() {
                AgentTriggerSource::Notification { source, .. } => Some(source.clone()),
                _ => None,
            })
    }

    /// Ends a subscription: an App event's listener by deleting its trigger, which disables its
    /// automation; any other through the ordinary detach.
    async fn end_notification_subscription(
        &self,
        subscription: &WorkflowNotificationSubscription,
        caller: &str,
    ) -> Result<(), DaemonError> {
        if let (Some(AgentTriggerNotificationSource::App { .. }), Some(trigger_id)) = (
            self.listened_source(subscription),
            subscription.trigger_id.clone(),
        ) {
            self.delete_agent_trigger(
                DeleteAgentTriggerRequest {
                    session_id: subscription.session_id.clone(),
                    trigger_id,
                },
                caller,
            )
            .await?;
            return Ok(());
        }
        self.execute_workflow_notification_command(
            LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
                session_id: subscription.session_id.clone(),
                subscription_id: subscription.subscription_id.clone(),
            }),
            caller,
        )
        .await?;
        Ok(())
    }

    /// Another agent of the same Room takes the subscription over with its prompt, options,
    /// counters and trace; the old trigger is removed. Every refusal comes before the first
    /// write, and a removal that fails puts the old subscription back as it was.
    async fn change_notification_subscriber(
        &self,
        row: NotificationSubscriptionRow,
        agent_id: String,
        caller: &str,
    ) -> Result<(), DaemonError> {
        let subscription = row.subscription;
        if subscription.target_kind != WorkflowNotificationTargetKind::Agent {
            return Err(store::error(
                "Only an agent's subscription changes subscriber. Remove this one and attach the workflow you want instead.",
            ));
        }
        if row.responsible_for.is_some() {
            return Err(store::error(RESPONSIBLE_LISTENER));
        }
        if row.source.is_none() {
            return Err(store::error(
                "The source is gone, so no new subscriber can attach to it. Remove this subscription.",
            ));
        }
        if subscription.agent_id.as_deref() == Some(agent_id.as_str()) {
            return Err(store::error(
                "This agent is already the subscriber. Pick another agent of the Room.",
            ));
        }
        let source = self.listened_source(&subscription).ok_or_else(|| {
            store::error("This subscription's trigger is gone. List the subscriptions again.")
        })?;
        let occupied = self
            .owned
            .session_store
            .get_session(&subscription.session_id)?
            .agent_triggers()
            .iter()
            .any(|trigger| {
                trigger.agent_id() == agent_id
                    && matches!(trigger.source(), AgentTriggerSource::Notification { source: s, .. } if *s == source)
            });
        if occupied {
            return Err(store::error(
                "That agent already subscribes to this source. Remove one of the two subscriptions first, then change the subscriber.",
            ));
        }
        let created = self
            .create_agent_trigger(
                CreateAgentTriggerRequest {
                    session_id: subscription.session_id.clone(),
                    agent_id,
                    source: AgentTriggerSourceRequest::Notification {
                        source,
                        options: row.options,
                        once: row.once,
                    },
                    busy_policy: None,
                    prompt: row.prompt,
                },
                caller,
            )
            .await?;
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = created else {
            return Err(store::error("unexpected agent trigger response"));
        };
        let to = notification_subscription_id(&trigger)
            .ok_or_else(|| store::error("the new subscriber's trigger has no subscription"))?
            .to_owned();
        let carry = |from: &str, to: &str| {
            self.owned
                .durable_state_store
                .notify(NotificationOperation::Carry {
                    from: from.to_owned(),
                    to: to.to_owned(),
                })
        };
        carry(&subscription.subscription_id, &to)?;
        if let Err(error) = self
            .end_notification_subscription(&subscription, caller)
            .await
        {
            carry(&to, &subscription.subscription_id)?;
            self.delete_agent_trigger(
                DeleteAgentTriggerRequest {
                    session_id: subscription.session_id.clone(),
                    trigger_id: trigger.id().to_owned(),
                },
                caller,
            )
            .await?;
            return Err(error);
        }
        Ok(())
    }
}

/// The most pressing thing about the row: a gone source or subscriber first, since nothing
/// can arrive until it is fixed.
fn health(
    source_present: bool,
    subscriber_present: bool,
    settings: &SubscriptionSettings,
) -> NotificationSubscriptionHealth {
    if !source_present {
        NotificationSubscriptionHealth::SourceGone
    } else if !subscriber_present {
        NotificationSubscriptionHealth::SubscriberGone
    } else if settings.options_changed {
        NotificationSubscriptionHealth::OptionsChanged
    } else if settings.loop_stopped_at_ms.is_some() {
        NotificationSubscriptionHealth::LoopStopped
    } else if settings.paused {
        NotificationSubscriptionHealth::Paused
    } else {
        NotificationSubscriptionHealth::Ok
    }
}
