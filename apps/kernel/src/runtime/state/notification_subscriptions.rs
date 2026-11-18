//! The Notification center's subscriptions table (docs/NOTIFICATION_CENTER_PLAN.md section 6):
//! every subscription whose subscriber lives on this kernel, in a Room the caller owns, with
//! what it picked, its counters and its health. Pause, change of subscriber, removal, the dry
//! run and the trace each act on one row of it.
use super::workflow_notification_router::{source_available, summaries};
use super::*;
use crate::durable_state::workflow_notifications::{
    self as store, subscription_settings::SubscriptionSettings, NotificationOperation,
};
use crate::local::{
    AgentTriggerSourceRequest, CreateAgentTriggerRequest, DetachWorkflowNotificationRequest,
    LocalDaemonResponse, NotificationSubscriptionChange, NotificationSubscriptionHealth,
    NotificationSubscriptionRow, NotificationTraceOutcome, PersonResponsibilityRow,
    UpdateNotificationSubscriptionRequest, WorkflowNotificationTargetKind,
};
use crate::session::AgentTriggerNotificationSource;

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
        self.execute_workflow_notification_command(
            LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
                session_id: row.subscription.session_id,
                subscription_id: row.subscription.subscription_id,
            }),
            caller,
        )
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
        let result = match self
            .owned
            .durable_state_store
            .notification_dry_run(&row.subscription)?
        {
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
        }
        Ok(rows)
    }

    /// Another agent of the same Room takes the subscription over with its prompt, options,
    /// counters and trace; the old trigger is removed. The source must still be there, since
    /// the new subscriber attaches to it.
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
        let created = self
            .create_agent_trigger(
                CreateAgentTriggerRequest {
                    session_id: subscription.session_id.clone(),
                    agent_id,
                    source: AgentTriggerSourceRequest::Notification {
                        source: AgentTriggerNotificationSource::Workflow {
                            source_id: subscription.source_id.clone(),
                        },
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
        if let Some(to) = super::agent_trigger_notification::notification_subscription_id(&trigger)
        {
            self.owned
                .durable_state_store
                .notify(NotificationOperation::Carry {
                    from: subscription.subscription_id.clone(),
                    to: to.to_owned(),
                })?;
        }
        self.execute_workflow_notification_command(
            LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
                session_id: subscription.session_id,
                subscription_id: subscription.subscription_id,
            }),
            caller,
        )
        .await?;
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
