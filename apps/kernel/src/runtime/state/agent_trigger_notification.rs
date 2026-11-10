//! An agent subscribes itself to a workflow exit or an App event. Every occurrence is queued
//! as a prompt of the agent; the subscription is the same row a workflow entry point uses,
//! with an agent target, so delivery, retention and loop prevention are shared.
use super::agent_trigger_runtime_state::AgentTriggerFiring;
use super::workflow_notification_router::{source_available, summaries};
use super::*;
use crate::durable_state::workflow_notifications::{
    self as store, NotificationOperation, NotificationOutcome,
};
use crate::local::{
    CreateAgentTriggerRequest, LocalDaemonResponse, WorkflowNotificationSubscription,
    WorkflowNotificationTargetKind,
};
use crate::session::{AgentTrigger, AgentTriggerNotificationSource, AgentTriggerSource};

/// An agent target has no queue of its own; the column keeps the shared row shape valid.
const AGENT_QUEUE_ID: &str = "agent";

/// A source the agent may subscribe to, as the picker names it.
pub(super) struct ResolvedSource {
    pub(in crate::runtime::state) kernel_id: String,
    pub(in crate::runtime::state) display_name: String,
}

impl KernelRuntimeState {
    pub(super) async fn create_notification_agent_trigger(
        &self,
        request: CreateAgentTriggerRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        let crate::local::AgentTriggerSourceRequest::Notification { source } = &request.source
        else {
            return Err(store::error("not a notification trigger"));
        };
        if request.busy_policy.is_some() {
            return Err(store::error(
                "a notification trigger always queues; it has no busy policy",
            ));
        }
        let agent = self.owned.ensure_agent_prompt_access(
            &request.agent_id,
            caller,
            "create agent trigger",
        )?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.agent_id,
            });
        }
        let home = self.owned.config_projection.snapshot().daemon_id;
        let owner = self
            .owned
            .session_store
            .get_session(&request.session_id)?
            .owner_user_id()
            .to_owned();
        let resolved = match source {
            AgentTriggerNotificationSource::Workflow { source_id } => self
                .resolve_workflow_notification_source(
                    &owner,
                    caller,
                    &request.session_id,
                    source_id,
                )?,
            AgentTriggerNotificationSource::App {
                installation_id,
                event_name,
            } => {
                self.resolve_app_notification_source(
                    &owner,
                    caller,
                    &request.session_id,
                    installation_id,
                    event_name,
                )
                .await?
            }
        };
        // The agent already listens to this workflow: subscribing again keeps the one listener
        // and takes the new prompt, so each completion reaches the agent once.
        let listener = match source {
            AgentTriggerNotificationSource::Workflow { .. } => self
                .owned
                .session_store
                .get_session(&request.session_id)?
                .agent_triggers()
                .iter()
                .find(|t| {
                    t.agent_id() == request.agent_id
                        && matches!(t.source(), AgentTriggerSource::Notification { source: s, .. } if s == source)
                })
                .map(|t| t.id().to_owned()),
            AgentTriggerNotificationSource::App { .. } => None,
        };
        if let Some(trigger_id) = listener {
            let trigger = self.owned.session_store.write().update_agent_trigger(
                &request.session_id,
                &trigger_id,
                None,
                request.prompt.filter(|p| !p.trim().is_empty()),
                Some(true),
            )?;
            let session = self
                .owned
                .persist_agent_trigger_session(&request.session_id, "agent_trigger_updated")?;
            return Ok(LocalDaemonResponse::AgentTriggerCreated { trigger, session });
        }
        self.authorize_current_external_command()?;
        let subscription_id = format!("wf_subscription_{:032x}", rand::random::<u128>());
        let trigger = self
            .owned
            .session_store
            .write()
            .create_agent_notification_trigger(
                &request.session_id,
                &request.agent_id,
                caller,
                source.clone(),
                subscription_id.clone(),
                resolved.display_name,
                request.prompt,
            )?;
        let subscription = if let AgentTriggerNotificationSource::Workflow { source_id } = source {
            Some(WorkflowNotificationSubscription {
                subscription_id,
                source_id: source_id.clone(),
                owner_user_id: owner.clone(),
                created_by_user_id: caller.into(),
                target_kernel_id: home.clone(),
                target_kind: WorkflowNotificationTargetKind::Agent,
                session_id: request.session_id.clone(),
                workflow_id: String::new(),
                publication_id: trigger.id().into(),
                endpoint_id: request.agent_id.clone(),
                queue_id: AGENT_QUEUE_ID.into(),
                source_available: true,
                source_kernel_id: resolved.kernel_id,
                agent_id: Some(request.agent_id.clone()),
                trigger_id: Some(trigger.id().into()),
            })
        } else {
            None
        };
        let subscribed = match &subscription {
            Some(subscription) => {
                self.subscribe_agent_notification(&home, subscription.clone())
                    .await
            }
            None => {
                self.subscribe_app_agent_notification(
                    &owner,
                    &request.session_id,
                    &request.agent_id,
                    &trigger,
                )
                .await
            }
        };
        if let Err(error) = subscribed.and_then(|_| self.authorize_current_external_command()) {
            self.owned
                .session_store
                .write()
                .delete_agent_trigger(&request.session_id, trigger.id())?;
            self.detach_deleted_notification_trigger(&trigger).await;
            return Err(error);
        }
        let session = match self
            .owned
            .persist_agent_trigger_session(&request.session_id, "agent_trigger_created")
        {
            Ok(session) => session,
            Err(error) => {
                self.owned
                    .session_store
                    .write()
                    .delete_agent_trigger(&request.session_id, trigger.id())?;
                self.detach_deleted_notification_trigger(&trigger).await;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::AgentTriggerCreated { trigger, session })
    }

    /// `<source> [prompt]` as the shell and the room command write it: a workflow source is named
    /// by its display name, `<kernel>/<name>` or its id (the name may hold spaces, so the longest
    /// run of leading words that names exactly one source wins), an App event is
    /// `app:<installation>/<event>`.
    pub(crate) fn choose_notification_source(
        &self,
        owner: &str,
        caller: &str,
        session_id: &str,
        words: &[String],
    ) -> Result<(AgentTriggerNotificationSource, Option<String>), DaemonError> {
        let prompt_after = |used: usize| {
            let prompt = words[used..].join(" ");
            (!prompt.is_empty()).then_some(prompt)
        };
        let Some(first) = words.first() else {
            return Err(store::error("name a notification source"));
        };
        if let Some(event) = first.strip_prefix("app:") {
            let (installation, event) = event
                .split_once('/')
                .filter(|(installation, event)| !installation.is_empty() && !event.is_empty())
                .ok_or_else(|| store::error("an App event is app:<installation>/<event>"))?;
            return Ok((
                AgentTriggerNotificationSource::App {
                    installation_id: installation.into(),
                    event_name: event.into(),
                },
                prompt_after(1),
            ));
        }
        let home = self.owned.config_projection.snapshot().daemon_id;
        let guest = !self.owned.acts_for_owner_in(owner, caller, owner)
            || self.room_provider_origin.is_some();
        let (local, _, _) = self
            .owned
            .durable_state_store
            .notification_inventory(owner)?;
        let mut sources = {
            let sessions = self.owned.session_store.read();
            summaries(
                &sessions,
                &local
                    .into_iter()
                    .filter(|s| s.enabled && source_available(&sessions, s))
                    .filter(|s| !guest || (s.session_id == session_id && s.kernel_id == home))
                    .collect::<Vec<_>>(),
            )
        };
        if !guest {
            sources.extend(
                self.owned
                    .durable_state_store
                    .notification_cached_sources(owner)?,
            );
        }
        for used in (1..=words.len()).rev() {
            let name = words[..used].join(" ");
            let mut named = sources.iter().filter(|s| {
                s.source_id == name
                    || s.display_name == name
                    || format!("{}/{}", s.kernel_id, s.display_name) == name
            });
            let Some(source) = named.next() else {
                continue;
            };
            if named.next().is_some() {
                return Err(store::error(format!(
                    "\"{name}\" names more than one source; use <kernel>/<name> or the source id"
                )));
            }
            if !source.available {
                return Err(store::error(format!(
                    "{} is not available: its kernel is offline",
                    source.display_name
                )));
            }
            return Ok((
                AgentTriggerNotificationSource::Workflow {
                    source_id: source.source_id.clone(),
                },
                prompt_after(used),
            ));
        }
        Err(store::error(format!(
            "no such notification source \"{}\"",
            words.join(" ")
        )))
    }

    /// The room owner's own sources of any kernel, or, for a collaborator, this room's sources.
    fn resolve_workflow_notification_source(
        &self,
        owner: &str,
        caller: &str,
        session_id: &str,
        source_id: &str,
    ) -> Result<ResolvedSource, DaemonError> {
        let home = self.owned.config_projection.snapshot().daemon_id;
        let guest = !self.owned.acts_for_owner_in(owner, caller, owner)
            || self.room_provider_origin.is_some();
        let (sources, _, _) = self
            .owned
            .durable_state_store
            .notification_inventory(owner)?;
        let sessions = self.owned.session_store.read();
        let local = sources.iter().find(|s| {
            s.source_id == source_id
                && s.enabled
                && source_available(&sessions, s)
                && (!guest || (s.session_id == session_id && s.kernel_id == home))
        });
        if let Some(source) = local {
            let named = summaries(&sessions, &sources)
                .into_iter()
                .find(|summary| summary.source_id == source.source_id)
                .ok_or_else(|| store::error("source not available"))?;
            return Ok(ResolvedSource {
                kernel_id: source.kernel_id.clone(),
                display_name: named.display_name,
            });
        }
        if guest {
            return Err(store::error("source not available"));
        }
        self.owned
            .durable_state_store
            .notification_cached_sources(owner)?
            .into_iter()
            .find(|s| s.source_id == source_id && s.available)
            .map(|s| ResolvedSource {
                kernel_id: s.kernel_id,
                display_name: s.display_name,
            })
            .ok_or_else(|| store::error("source not available"))
    }

    /// Saves the subscription here, and on the source's kernel when it is another one.
    pub(super) async fn subscribe_agent_notification(
        &self,
        home: &str,
        subscription: WorkflowNotificationSubscription,
    ) -> Result<(), DaemonError> {
        let remote = subscription.source_kernel_id != home;
        let (subscription, generation) = match self.owned.durable_state_store.notify(if remote {
            NotificationOperation::AttachToRemoteSource { subscription }
        } else {
            NotificationOperation::RemoteAttach { subscription }
        })? {
            NotificationOutcome::RemoteInstalling {
                subscription,
                generation,
            } => (subscription, generation),
            NotificationOutcome::Subscription(_) if !remote => return Ok(()),
            _ => return Err(store::error("unexpected notification response")),
        };
        if self.install_on_source(&subscription, generation).await? {
            return Ok(());
        }
        Err(store::error(
            "source not available; refresh sources and retry",
        ))
    }

    /// Ends the subscription of a trigger that is gone: here, and on the source's kernel.
    pub(super) async fn detach_agent_notification(
        &self,
        subscription: &WorkflowNotificationSubscription,
    ) {
        let _ = self.detach_notification_subscription(subscription).await;
    }

    /// A deleted notification trigger stops receiving at once; other ways a trigger can
    /// vanish (archive, removal, room end) are caught by the pump's reconcile.
    pub(super) async fn detach_deleted_notification_trigger(&self, trigger: &AgentTrigger) {
        if matches!(
            trigger.source(),
            AgentTriggerSource::Notification {
                source: AgentTriggerNotificationSource::App { .. },
                ..
            }
        ) {
            self.detach_app_agent_notification(trigger).await;
            return;
        }
        let Some(subscription_id) = notification_subscription_id(trigger) else {
            return;
        };
        let subscription = self
            .agent_notification_subscriptions()
            .into_iter()
            .find(|s| s.subscription_id == subscription_id);
        if let Some(subscription) = subscription {
            self.detach_agent_notification(&subscription).await;
        }
    }

    /// The active agent-target subscriptions of this kernel.
    pub(super) fn agent_notification_subscriptions(&self) -> Vec<WorkflowNotificationSubscription> {
        let home = self.owned.config_projection.snapshot().daemon_id;
        let owners: BTreeSet<String> = self
            .owned
            .session_store
            .read()
            .list_sessions()
            .iter()
            .map(|session| session.owner_user_id().to_owned())
            .collect();
        owners
            .iter()
            .filter_map(|owner| {
                self.owned
                    .durable_state_store
                    .notification_inventory(owner)
                    .ok()
            })
            .flat_map(|(_, subscriptions, _)| subscriptions)
            .filter(|s| {
                s.target_kind == WorkflowNotificationTargetKind::Agent && s.target_kernel_id == home
            })
            .collect()
    }

    /// One pump pass: end the subscriptions whose trigger or agent is gone (archived, removed,
    /// room ended, collaborator left), then queue the accepted occurrences of the live ones.
    pub(super) async fn route_agent_notifications(&self) {
        if !self.owned.publication_activation.is_active()
            || self
                .owned
                .durable_state_store
                .require_writer_healthy()
                .is_err()
        {
            return;
        }
        let sessions = self.owned.session_store.read().list_sessions();
        for session in sessions {
            for agent in session.agents() {
                if let Err(error) = self.owned.expire_queued_agent_notifications(
                    session.id(),
                    agent.id(),
                    crate::session::unix_epoch_ms(),
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.agent_notification",
                        "notification expiry deferred",
                        serde_json::json!({"error":error.to_string()}),
                    );
                }
            }
        }
        let subscriptions = self.agent_notification_subscriptions();
        if subscriptions.is_empty() {
            return;
        }
        let mut live = Vec::new();
        for subscription in subscriptions {
            if self
                .owned
                .session_store
                .get_session(&subscription.session_id)
                .is_ok_and(|session| {
                    self.owned
                        .agent_notification_target_live(&session, &subscription)
                })
            {
                live.push(subscription);
            } else {
                self.detach_agent_notification(&subscription).await;
            }
        }
        let subscriptions = one_listener_per_agent_and_source(live);
        let now = crate::session::unix_epoch_ms();
        let Ok(candidates) = self
            .owned
            .durable_state_store
            .notification_candidates(true, now, 8)
        else {
            return;
        };
        for (subscription, envelope) in candidates {
            if subscription.target_kind != WorkflowNotificationTargetKind::Agent
                || !subscriptions
                    .iter()
                    .any(|s| s.subscription_id == subscription.subscription_id)
            {
                continue;
            }
            // Rotate first, so a target that cannot take the prompt never starves the others.
            let _ = self
                .owned
                .durable_state_store
                .notify(NotificationOperation::Retry {
                    subscription_id: subscription.subscription_id.clone(),
                    source_id: envelope.source_id.clone(),
                    occurrence_id: envelope.occurrence_id.clone(),
                    accepted: true,
                    at: now.saturating_add(1000),
                });
            if !self
                .owned
                .session_store
                .get_session(&subscription.session_id)
                .is_ok_and(|session| {
                    self.owned
                        .agent_notification_target_live(&session, &subscription)
                })
            {
                continue;
            }
            if let Err(error) = self.queue_agent_notification(&subscription, envelope).await {
                crate::logging::error_with_fields(
                    "daemon.agent_notification",
                    "an accepted notification could not be queued",
                    serde_json::json!({
                        "subscription_id": subscription.subscription_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    async fn queue_agent_notification(
        &self,
        subscription: &WorkflowNotificationSubscription,
        envelope: crate::local::WorkflowNotificationEnvelope,
    ) -> Result<(), DaemonError> {
        let session_id = &subscription.session_id;
        let trigger_id = subscription
            .trigger_id
            .as_deref()
            .ok_or_else(|| store::error("notification identifier invalid"))?;
        let payload = serde_json::to_value(&envelope)
            .map_err(|_| store::error("notification encoding failed"))?;
        let receipt =
            crate::durable_state::workflow_notifications::receipt_id(subscription, &envelope)?;
        if let Some(prompt_id) = self
            .queue_agent_notification_input(session_id, trigger_id, &receipt, payload)
            .await?
        {
            self.settle_agent_notification(subscription, &envelope, &prompt_id)?;
        }
        Ok(())
    }

    /// Admits the prompt for one accepted receipt, or returns the prompt admitted for it
    /// before a restart. The receipt's lineage commits with the admission itself; the
    /// caller settles the receipt afterwards.
    pub(super) async fn queue_agent_notification_input(
        &self,
        session_id: &str,
        trigger_id: &str,
        receipt_id: &str,
        payload: serde_json::Value,
    ) -> Result<Option<String>, DaemonError> {
        let now = crate::session::unix_epoch_ms();
        let deadline = payload
            .get("deadline_ms")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| store::error("notification deadline missing"))?;
        let session = self.owned.session_store.get_session(session_id)?;
        let agent_id = session
            .agent_triggers()
            .iter()
            .find(|trigger| trigger.id() == trigger_id)
            .ok_or_else(|| store::error("notification trigger missing"))?
            .agent_id();
        self.owned
            .expire_queued_agent_notifications(session_id, agent_id, now)?;
        // Admission fence: a receipt that already names its prompt was admitted durably.
        // Whether that prompt is still queued or already ran, was steered or was cancelled,
        // the occurrence is delivered once; the caller settles it with that prompt.
        if let Some(prompt_id) = self
            .owned
            .durable_state_store
            .admitted_agent_notification_prompt(receipt_id)?
        {
            return Ok(Some(prompt_id));
        }
        if now >= deadline {
            return Ok(None);
        }
        self.owned
            .durable_state_store
            .reserve_agent_notification_lineage(
                receipt_id,
                &crate::durable_state::workflow_notifications::agent_lineage::operation_id(
                    trigger_id, &payload,
                )?,
            )?;
        let Some(dispatch) = self.owned.session_store.write().claim_input_agent_trigger(
            session_id,
            trigger_id,
            crate::session::TriggerInputKind::Notification,
            payload,
        )?
        else {
            // Off, or mid-firing: the accepted occurrence waits for the next pass.
            return Ok(None);
        };
        let now = crate::session::unix_epoch_ms();
        match self.fire_agent_trigger(&dispatch).await {
            Ok(AgentTriggerFiring::Fired {
                prompt_id,
                outcome,
                steer_refusal,
            }) => {
                self.owned.session_store.write().mark_agent_trigger_fired(
                    session_id,
                    trigger_id,
                    now,
                    outcome,
                    steer_refusal,
                )?;
                self.owned
                    .persist_agent_trigger_session(session_id, "agent_trigger_invoked")?;
                Ok(Some(prompt_id))
            }
            Ok(AgentTriggerFiring::Settled { prompt_id }) => Ok(Some(prompt_id)),
            Ok(
                AgentTriggerFiring::Skipped
                | AgentTriggerFiring::Coalesced
                | AgentTriggerFiring::Deferred
                | AgentTriggerFiring::AgentGone,
            ) => {
                self.owned
                    .session_store
                    .write()
                    .defer_agent_trigger(session_id, trigger_id, now)?;
                Ok(None)
            }
            Err(error) => {
                self.owned.session_store.write().mark_agent_trigger_failed(
                    session_id,
                    trigger_id,
                    now,
                    error.to_string(),
                )?;
                Err(error)
            }
        }
    }

    fn settle_agent_notification(
        &self,
        subscription: &WorkflowNotificationSubscription,
        envelope: &crate::local::WorkflowNotificationEnvelope,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .notify(NotificationOperation::AgentQueued {
                subscription_id: subscription.subscription_id.clone(),
                source_id: envelope.source_id.clone(),
                occurrence_id: envelope.occurrence_id.clone(),
                prompt_id: prompt_id.into(),
                session_id: subscription.session_id.clone(),
            })
            .map(|_| ())
    }
}

impl KernelRuntimeOwnedState {
    /// The agent of a subscription is present and its notification trigger exists in `session`,
    /// the subscription's room. The caller reads the room once, so this never locks the session
    /// store: a second read under a held one waits for ever once a writer queues between them.
    pub(super) fn agent_notification_target_live(
        &self,
        session: &crate::session::RuntimeSession,
        subscription: &WorkflowNotificationSubscription,
    ) -> bool {
        session.id() == subscription.session_id
            && session.status() != crate::session::SessionStatus::Ended
            && session.has_member(&subscription.created_by_user_id)
            && session.owner_user_id() == subscription.owner_user_id
            && self
                .agent_store
                .get_agent(&subscription.endpoint_id)
                .is_ok_and(|agent| {
                    !agent.archived() && agent.session_id() == subscription.session_id
                })
            && session.agent_triggers().iter().any(|trigger| {
                Some(trigger.id()) == subscription.trigger_id.as_deref()
                    && trigger.agent_id() == subscription.endpoint_id
                    && matches!(
                        trigger.source(),
                        AgentTriggerSource::Notification { subscription_id, .. }
                            if *subscription_id == subscription.subscription_id
                    )
            })
    }
}

/// A store written before subscribing twice was idempotent may hold several listeners of one
/// agent on one source; only the first (by id) delivers, so an occurrence reaches the agent once.
fn one_listener_per_agent_and_source(
    mut subscriptions: Vec<WorkflowNotificationSubscription>,
) -> Vec<WorkflowNotificationSubscription> {
    subscriptions.sort_by(|a, b| a.subscription_id.cmp(&b.subscription_id));
    let mut seen = BTreeSet::new();
    subscriptions.retain(|s| seen.insert((s.endpoint_id.clone(), s.source_id.clone())));
    subscriptions
}

/// The notification subscription of a trigger, if it is one.
pub(super) fn notification_subscription_id(trigger: &AgentTrigger) -> Option<&str> {
    match trigger.source() {
        AgentTriggerSource::Notification {
            subscription_id, ..
        } => Some(subscription_id),
        _ => None,
    }
}

#[cfg(test)]
#[path = "agent_trigger_notification_tests.rs"]
mod tests;
