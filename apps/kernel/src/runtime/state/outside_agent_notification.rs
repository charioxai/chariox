//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17, ON-18): an outside agent's grant follows the owner's
//! notification sources on this kernel. Occurrences reach the grant's own event stream and
//! nowhere else; the grant, never a destination, is the subscriber.
use super::*;
use crate::durable_state::workflow_notifications::{
    outside_agent::MAX_FOLLOWED_SOURCES, NotificationOperation, NotificationOutcome,
};
use crate::local::*;

fn refused(message: &'static str) -> DaemonError {
    crate::durable_state::agent_lifecycle::outside::refused(message)
}

impl KernelRuntimeState {
    /// Makes `grant` the subscriber of each source, or of none: every source is checked first.
    /// A source the owner cannot see, on another kernel or already gone is "not available",
    /// whether or not its id exists.
    pub(crate) fn follow_notification_sources(
        &self,
        grant: &str,
        owner: &str,
        source_ids: &[String],
    ) -> Result<(), DaemonError> {
        if source_ids.len() as i64 > MAX_FOLLOWED_SOURCES {
            return Err(refused("event source limit"));
        }
        if self.access_grant_summary(grant).is_none() {
            return Err(refused(
                "notification sources are followed by an access grant",
            ));
        }
        let home = self.owned.config_projection.snapshot().daemon_id;
        self.owned
            .durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let sessions = self.owned.session_store.read();
                let mut subscriptions = Vec::new();
                for source_id in source_ids {
                    let source = self
                        .attachable_source(&sessions, owner, source_id, true)
                        .ok()
                        .filter(|source| source.kernel_id == home)
                        .ok_or_else(super::source_not_available)?;
                    subscriptions.push(WorkflowNotificationSubscription {
                        subscription_id: format!("wf_subscription_{:032x}", rand::random::<u128>()),
                        source_id: source.source_id,
                        owner_user_id: owner.into(),
                        created_by_user_id: owner.into(),
                        target_kernel_id: home.clone(),
                        target_kind: WorkflowNotificationTargetKind::OutsideAgent,
                        session_id: source.session_id,
                        workflow_id: String::new(),
                        publication_id: grant.into(),
                        endpoint_id: grant.into(),
                        queue_id: "grant".into(),
                        source_available: true,
                        source_kernel_id: home.clone(),
                        agent_id: None,
                        trigger_id: None,
                    });
                }
                for subscription in subscriptions {
                    match self
                        .owned
                        .durable_state_store
                        .notify(NotificationOperation::Granted {
                            grant: grant.into(),
                            attach: Box::new(NotificationOperation::RemoteAttach { subscription }),
                        }) {
                        Ok(NotificationOutcome::Subscription(_)) => {}
                        Ok(_) => return Err(refused("unexpected notification response")),
                        Err(error) => return Err(public_follow_error(error)),
                    }
                }
                Ok(())
            })?;
        // A grant that ended while attaching takes what it attached with it.
        if self.access_grant_summary(grant).is_none() {
            self.retire_notification_grants(Some(grant));
            return Err(refused("grant revoked or expired"));
        }
        Ok(())
    }

    /// Ends the grant's follow of the named sources, or of all of them. Occurrences already in
    /// the grant's stream stay readable until acknowledged. Naming a source it does not follow
    /// changes nothing.
    pub(crate) fn unfollow_notification_sources(
        &self,
        grant: &str,
        owner: &str,
        sources: Option<&[String]>,
    ) -> Result<(), DaemonError> {
        let home = self.owned.config_projection.snapshot().daemon_id;
        let (_, subscriptions, _) = self
            .owned
            .durable_state_store
            .notification_inventory(owner)?;
        for sub in subscriptions.into_iter().filter(|sub| {
            sub.target_kind == WorkflowNotificationTargetKind::OutsideAgent
                && sub.endpoint_id == grant
                && sources.is_none_or(|named| named.contains(&sub.source_id))
        }) {
            self.owned
                .durable_state_store
                .notify(NotificationOperation::Detach {
                    subscription_id: sub.subscription_id,
                    owner: owner.into(),
                    kernel: home.clone(),
                })?;
        }
        Ok(())
    }

    /// The sources a grant of `owner` can follow on this kernel: exactly those
    /// `follow_notification_sources` accepts. "Agent needs your action" is created with its Room's
    /// first listing, so it is ensured here and can be followed before any task has blocked.
    pub(crate) fn followable_notification_sources(
        &self,
        owner: &str,
    ) -> Result<Vec<WorkflowNotificationSourceSummary>, DaemonError> {
        for session in self.owned.session_store.list_sessions() {
            if session.owner_user_id() == owner
                && session.status() != crate::session::SessionStatus::Ended
            {
                self.owned
                    .ensure_owner_action_notification_source(&session)?;
            }
        }
        let home = self.owned.config_projection.snapshot().daemon_id;
        let (mut sources, _, _) = self
            .owned
            .durable_state_store
            .notification_inventory(owner)?;
        let sessions = self.owned.session_store.read();
        sources.retain(|source| {
            source.kernel_id == home
                && source.enabled
                && super::workflow_notification_router::source_available(&sessions, source)
        });
        for source in &mut sources {
            source.available = true;
        }
        Ok(super::workflow_notification_router::summaries(
            &sessions, &sources,
        ))
    }

    /// What the grant follows, as `GetEventSubscription` shows it.
    pub(crate) fn followed_notification_sources(
        &self,
        grant: &str,
    ) -> Result<Vec<EventSourceFollow>, DaemonError> {
        let Some(owner) = self.event_source_owner(grant) else {
            return Ok(Vec::new());
        };
        let (sources, subscriptions, _) = self
            .owned
            .durable_state_store
            .notification_inventory(&owner)?;
        let followed: Vec<_> = subscriptions
            .into_iter()
            .filter(|sub| {
                sub.target_kind == WorkflowNotificationTargetKind::OutsideAgent
                    && sub.endpoint_id == grant
            })
            .collect();
        if followed.is_empty() {
            return Ok(Vec::new());
        }
        let sessions = self.owned.session_store.read();
        let mut sources = sources;
        for source in &mut sources {
            source.available =
                super::workflow_notification_router::source_available(&sessions, source);
        }
        let summaries = super::workflow_notification_router::summaries(&sessions, &sources);
        Ok(followed
            .into_iter()
            .filter_map(|sub| {
                let summary = summaries
                    .iter()
                    .find(|summary| summary.source_id == sub.source_id)?;
                Some(EventSourceFollow {
                    source_id: summary.source_id.clone(),
                    display_name: summary.display_name.clone(),
                    kind: summary.kind,
                    session_id: summary.session_id.clone(),
                    subscription_id: sub.subscription_id,
                    available: summary.available,
                })
            })
            .collect())
    }
}

/// Store errors reach an outside caller as constant sentences, never as database text.
fn public_follow_error(error: DaemonError) -> DaemonError {
    match error.to_string() {
        message if message.contains("event source limit") => refused("event source limit"),
        message if message.contains("notification fanout limit") => {
            refused("notification fanout limit")
        }
        message if message.contains("workflow notification subscription limit") => {
            refused("notification subscription limit")
        }
        _ => super::source_not_available(),
    }
}

impl KernelRuntimeOwnedState {
    /// Delivers one occurrence to the grant that follows its source: accepted (deduplicated,
    /// dated, checked against loops) and appended to the grant's stream in one commit. An
    /// occurrence of a source that is gone, or of a grant that ended, is left to expire.
    pub(super) fn deliver_outside_agent_notification(
        &self,
        sub: &WorkflowNotificationSubscription,
        env: &WorkflowNotificationEnvelope,
        accepted: bool,
    ) {
        let _ = self
            .durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                self.with_live_notification_grant(sub, |require_grant| {
                    let sessions = self.session_store.read();
                    let (sources, _, _) = self
                        .durable_state_store
                        .notification_inventory(&sub.owner_user_id)?;
                    if !sources.iter().any(|source| {
                        source.source_id == env.source_id
                            && super::workflow_notification_router::source_available(
                                &sessions, source,
                            )
                    }) {
                        return Ok(());
                    }
                    require_grant()?;
                    if !accepted {
                        let NotificationOutcome::Ack(ack) =
                            self.durable_state_store
                                .notify(NotificationOperation::Accept {
                                    subscription: sub.clone(),
                                    envelope: env.clone(),
                                })?
                        else {
                            return Ok(());
                        };
                        if ack != WorkflowNotificationAck::Accepted {
                            self.durable_state_store.notify(
                                NotificationOperation::Acknowledge {
                                    subscription_id: sub.subscription_id.clone(),
                                    occurrence_id: env.occurrence_id.clone(),
                                },
                            )?;
                            return Ok(());
                        }
                    }
                    require_grant()?;
                    self.durable_state_store
                        .notify(NotificationOperation::AppendToGrantInbox {
                            subscription: sub.clone(),
                            envelope: env.clone(),
                        })?;
                    Ok(())
                })
            });
    }
}
