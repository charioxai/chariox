//! An agent subscribes itself to a workflow exit or an App event. Every occurrence is queued
//! as a prompt of the agent; the subscription is the same row a workflow entry point uses,
//! with an agent target, so delivery, retention and loop prevention are shared.
use super::agent_trigger_runtime_state::AgentTriggerFiring;
use super::workflow_notification_peers::VERSION;
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
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

/// An agent target has no queue of its own; the column keeps the shared row shape valid.
const AGENT_QUEUE_ID: &str = "agent";

/// A source the agent may subscribe to, as the picker names it.
struct ResolvedSource {
    kernel_id: String,
    display_name: String,
}

impl KernelRuntimeState {
    pub(super) async fn create_notification_agent_trigger(
        &self,
        request: CreateAgentTriggerRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
            AgentTriggerNotificationSource::App { .. } => {
                return Err(store::error(
                    "App notification triggers are not available yet",
                ))
            }
        };
        let AgentTriggerNotificationSource::Workflow { source_id } = source else {
            unreachable!("App sources returned above")
        };
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
        let subscription = WorkflowNotificationSubscription {
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
        };
        if let Err(error) = self
            .subscribe_agent_notification(&owner, &home, subscription.clone())
            .await
        {
            self.owned
                .session_store
                .write()
                .delete_agent_trigger(&request.session_id, trigger.id())?;
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
                self.detach_agent_notification(&subscription).await;
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
        let guest = caller != owner;
        let (local, _, _) = self.owned.durable_state_store.notification_inventory(owner)?;
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
            sources.extend(self.owned.durable_state_store.notification_cached_sources(owner)?);
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
        let guest = caller != owner;
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
    async fn subscribe_agent_notification(
        &self,
        owner: &str,
        home: &str,
        subscription: WorkflowNotificationSubscription,
    ) -> Result<(), DaemonError> {
        let NotificationOutcome::Subscription(subscription) = self
            .owned
            .durable_state_store
            .notify(NotificationOperation::RemoteAttach { subscription })?
        else {
            return Err(store::error("unexpected notification response"));
        };
        if subscription.source_kernel_id == home {
            return Ok(());
        }
        let response = self
            .notification_peer(
                owner,
                &subscription.source_kernel_id,
                RelayPeerRequest::SubscribeWorkflowNotifications {
                    protocol_version: VERSION,
                    source_id: subscription.source_id.clone(),
                    target_ref: subscription.clone(),
                },
            )
            .await;
        if matches!(&response, Ok(RelayPeerResponse::WorkflowNotificationSubscribed { subscription: s }) if *s == subscription)
        {
            return Ok(());
        }
        self.owned
            .durable_state_store
            .notify(NotificationOperation::Detach {
                subscription_id: subscription.subscription_id,
                owner: owner.into(),
                kernel: subscription.target_kernel_id,
            })?;
        Err(store::error(
            "source not available; refresh sources and retry",
        ))
    }

    /// Ends the subscription of a trigger that is gone: here, and on the source's kernel.
    pub(super) async fn detach_agent_notification(
        &self,
        subscription: &WorkflowNotificationSubscription,
    ) {
        let _ = self
            .owned
            .durable_state_store
            .notify(NotificationOperation::Detach {
                subscription_id: subscription.subscription_id.clone(),
                owner: subscription.owner_user_id.clone(),
                kernel: subscription.target_kernel_id.clone(),
            });
        if subscription.source_kernel_id != subscription.target_kernel_id {
            let _ = self
                .notification_peer(
                    &subscription.owner_user_id,
                    &subscription.source_kernel_id,
                    RelayPeerRequest::UnsubscribeWorkflowNotifications {
                        protocol_version: VERSION,
                        subscription_id: subscription.subscription_id.clone(),
                    },
                )
                .await;
        }
    }

    /// A deleted notification trigger stops receiving at once; other ways a trigger can
    /// vanish (archive, removal, room end) are caught by the pump's reconcile.
    pub(super) async fn detach_deleted_notification_trigger(&self, trigger: &AgentTrigger) {
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
    fn agent_notification_subscriptions(&self) -> Vec<WorkflowNotificationSubscription> {
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
        let subscriptions = self.agent_notification_subscriptions();
        if subscriptions.is_empty() {
            return;
        }
        for subscription in &subscriptions {
            if !self.owned.agent_notification_target_live(subscription) {
                self.detach_agent_notification(subscription).await;
            }
        }
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
            if !self.owned.agent_notification_target_live(&subscription) {
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
        let payload = serde_json::json!({
            "source": envelope.display_name,
            "source_id": envelope.source_id,
            "occurrence_id": envelope.occurrence_id,
            "output": envelope.output,
        });
        let Some(dispatch) = self.owned.session_store.write().claim_input_agent_trigger(
            session_id,
            trigger_id,
            crate::session::TriggerInputKind::Notification,
            payload,
        )?
        else {
            // Off, or mid-firing: the accepted occurrence waits for the next pass.
            return Ok(());
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
                self.settle_agent_notification(subscription, &envelope, &prompt_id)
            }
            Ok(AgentTriggerFiring::Settled { prompt_id }) => {
                self.settle_agent_notification(subscription, &envelope, &prompt_id)
            }
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
                Ok(())
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
    /// The agent of a subscription is present and its notification trigger exists.
    pub(super) fn agent_notification_target_live(
        &self,
        subscription: &WorkflowNotificationSubscription,
    ) -> bool {
        let Ok(session) = self.session_store.get_session(&subscription.session_id) else {
            return false;
        };
        session.owner_user_id() == subscription.owner_user_id
            && self
                .agent_store
                .get_agent(&subscription.endpoint_id)
                .is_ok()
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
mod tests {
    use super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt;
    use super::*;
    use crate::local::{
        AgentTriggerSourceRequest, DeleteAgentTriggerRequest, ListWorkflowNotificationsRequest,
        LocalDaemonRequest, RegisterWorkflowNotificationSourceRequest, UpdateAgentTriggerRequest,
    };
    use crate::session::{
        AgentTriggerBusyPolicy, WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowOutputPayload,
        WorkflowRun, WorkflowRunStatus,
    };

    struct Rig {
        _root: crate::test_support::TestWorktree,
        runtime: KernelRuntimeState,
        session_id: String,
        agent_id: String,
    }

    async fn rig() -> Rig {
        let (root, runtime, session_id, agent_id, _, _, _) = runtime_with_active_prompt().await;
        Rig {
            _root: root,
            runtime,
            session_id,
            agent_id,
        }
    }

    impl Rig {
        /// A workflow whose one node is its exit; returns the workflow id.
        fn workflow_with_exit(&self, name: &str) -> String {
            let mut sessions = self.runtime.owned.session_store.write();
            let workflow = sessions
                .create_workflow(&self.session_id, Some(name.into()))
                .unwrap();
            let node = sessions
                .add_workflow_node(&self.session_id, workflow.id(), "agent")
                .unwrap();
            let mut session = sessions.get_session(&self.session_id).unwrap();
            session
                .workflow_mut(workflow.id())
                .unwrap()
                .node_mut(node.id())
                .unwrap()
                .set_can_complete_workflow_run(true);
            sessions.restore_session(session.clone());
            drop(sessions);
            self.runtime
                .owned
                .durable_state_store
                .persist_workflow_runtime_transition(&session, "agent notification rig")
                .unwrap();
            workflow.id().into()
        }

        fn register(&self, workflow: &str) -> String {
            let response = self
                .runtime
                .execute_workflow_notification_request(
                    LocalDaemonRequest::RegisterWorkflowNotificationSource(
                        RegisterWorkflowNotificationSourceRequest {
                            session_id: self.session_id.clone(),
                            workflow_ref: workflow.into(),
                            exit_node_id: "exit-1".into(),
                            enabled: true,
                        },
                    ),
                    "local",
                )
                .unwrap();
            let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = response
            else {
                panic!()
            };
            source.source_id
        }

        fn complete(&self, workflow: &str, run_id: &str, message: &str) {
            let session = self
                .runtime
                .owned
                .session_store
                .get_session(&self.session_id)
                .unwrap();
            let exit = session
                .workflow(workflow)
                .unwrap()
                .nodes()
                .iter()
                .find(|node| node.can_complete_workflow_run())
                .unwrap()
                .id()
                .to_owned();
            let node_run = format!("node-run-{run_id}");
            let mut run = WorkflowRun::new(
                run_id,
                workflow,
                "endpoint",
                "node",
                None,
                None,
                vec![WorkflowNodeRun::new(
                    &node_run,
                    exit,
                    "agent",
                    0,
                    WorkflowNodeRunStatus::Completed,
                )],
                vec![],
            );
            run.set_invocation_context(0, None, None, 0, None);
            run.set_final_output(
                Some(WorkflowOutputPayload::new(message, vec![])),
                Some(true),
                None,
                Some(node_run),
            );
            run.set_status(WorkflowRunStatus::Completed);
            let mut session = session;
            session.create_workflow_run(run);
            self.runtime
                .owned
                .session_store
                .write()
                .restore_session(session.clone());
            self.runtime
                .owned
                .durable_state_store
                .persist_workflow_runtime_transition(&session, "agent notification rig run")
                .unwrap();
        }

        fn request(&self, source_id: &str) -> CreateAgentTriggerRequest {
            CreateAgentTriggerRequest {
                session_id: self.session_id.clone(),
                agent_id: self.agent_id.clone(),
                source: AgentTriggerSourceRequest::Notification {
                    source: AgentTriggerNotificationSource::Workflow {
                        source_id: source_id.into(),
                    },
                },
                busy_policy: None,
                prompt: Some("Triage what the pipeline reports".into()),
            }
        }

        async fn pass(&self) {
            self.runtime.owned.route_workflow_notifications();
            self.runtime.route_agent_notifications().await;
            // Accepted occurrences are queued by the pass after the one that accepted them.
            self.runtime.owned.route_workflow_notifications();
            self.runtime.route_agent_notifications().await;
        }

        fn queued_prompts(&self) -> Vec<String> {
            let session = self
                .runtime
                .owned
                .session_snapshot(&self.session_id)
                .unwrap();
            let (_, queued) = self
                .runtime
                .owned
                .prompt_state_owner
                .state_parts(&session, &self.agent_id);
            queued.iter().map(|p| p.prompt().to_owned()).collect()
        }

        fn subscriptions(&self) -> Vec<WorkflowNotificationSubscription> {
            self.runtime.agent_notification_subscriptions()
        }
    }

    // MP-08 / MP-10 (VS-S2-2): every occurrence is queued, none coalesced.
    #[tokio::test]
    async fn an_agent_notification_trigger_queues_every_occurrence() {
        let rig = rig().await;
        let workflow = rig.workflow_with_exit("pipeline");
        let source = rig.register(&workflow);

        let mut refused = rig.request(&source);
        refused.busy_policy = Some(AgentTriggerBusyPolicy::Steer);
        assert!(rig
            .runtime
            .create_agent_trigger(refused, "local")
            .await
            .is_err());
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = rig
            .runtime
            .create_agent_trigger(rig.request(&source), "local")
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(trigger.busy_policy(), AgentTriggerBusyPolicy::Queue);
        assert!(matches!(
            trigger.source(),
            AgentTriggerSource::Notification { display_name, .. } if display_name == "pipeline"
        ));
        assert!(rig
            .runtime
            .update_agent_trigger(
                UpdateAgentTriggerRequest {
                    session_id: rig.session_id.clone(),
                    trigger_id: trigger.id().into(),
                    busy_policy: Some(AgentTriggerBusyPolicy::Steer),
                    prompt: None,
                    enabled: None,
                },
                "local",
            )
            .await
            .is_err());

        for n in 0..3 {
            rig.complete(
                &workflow,
                &format!("run-{n}"),
                &format!("release {n} approved"),
            );
        }
        rig.pass().await;
        let queued = rig.queued_prompts();
        assert_eq!(queued.len(), 3, "{queued:?}");
        for (n, text) in queued.iter().enumerate() {
            assert!(text.starts_with(
                "Triage what the pipeline reports\n\n<chariox-trigger-input kind=\"notification\">"
            ));
            assert!(text.contains(&format!("release {n} approved")), "{text}");
            assert!(text.contains("\"source\":\"pipeline\""), "{text}");
        }
        let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
        assert_eq!(session.agent_triggers()[0].fired(), 3);
        assert_eq!(session.agent_triggers()[0].coalesced(), 0);
        // Settled receipts are not delivered twice.
        rig.pass().await;
        assert_eq!(rig.queued_prompts().len(), 3);
    }

    // MP-08 / MP-10 (VS-S2-5): a subscription lasts as long as its trigger and agent.
    #[tokio::test]
    async fn deleting_the_trigger_or_losing_the_agent_ends_the_subscription() {
        for removal in ["delete", "agent_trigger_removed", "agent_archived"] {
            let rig = rig().await;
            let workflow = rig.workflow_with_exit("pipeline");
            let source = rig.register(&workflow);
            let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = rig
                .runtime
                .create_agent_trigger(rig.request(&source), "local")
                .await
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(rig.subscriptions().len(), 1, "{removal}");
            match removal {
                "delete" => {
                    rig.runtime
                        .delete_agent_trigger(
                            DeleteAgentTriggerRequest {
                                session_id: rig.session_id.clone(),
                                trigger_id: trigger.id().into(),
                            },
                            "local",
                        )
                        .await
                        .unwrap();
                }
                "agent_trigger_removed" => {
                    rig.runtime
                        .owned
                        .session_store
                        .write()
                        .remove_agent_triggers_for_agent(&rig.session_id, &rig.agent_id)
                        .unwrap();
                }
                _ => {
                    rig.runtime
                        .set_agent_archived_without_cleanup_for_test(&rig.agent_id, true);
                    rig.runtime
                        .owned
                        .session_store
                        .write()
                        .remove_agent_triggers_for_agent(&rig.session_id, &rig.agent_id)
                        .unwrap();
                }
            }
            rig.complete(&workflow, "after", "released");
            rig.pass().await;
            assert!(rig.subscriptions().is_empty(), "{removal}");
            assert!(rig.queued_prompts().is_empty(), "{removal}");
            let LocalDaemonResponse::WorkflowNotifications { subscriptions, .. } = rig
                .runtime
                .execute_workflow_notification_request(
                    LocalDaemonRequest::ListWorkflowNotifications(
                        ListWorkflowNotificationsRequest {
                            session_id: rig.session_id.clone(),
                        },
                    ),
                    "local",
                )
                .unwrap()
            else {
                panic!()
            };
            assert!(subscriptions.is_empty(), "{removal}");
        }
    }

    // MP-08 / MP-10 / MP-11 (VS-S1-6, VS-S4-3): a collaborator's agent sees this room's sources only.
    #[tokio::test]
    async fn a_collaborator_resolves_only_this_rooms_sources() {
        let rig = rig().await;
        let workflow = rig.workflow_with_exit("pipeline");
        let source = rig.register(&workflow);
        let owner = "local";
        assert!(rig
            .runtime
            .resolve_workflow_notification_source(owner, "guest", &rig.session_id, &source)
            .is_ok());
        assert!(
            rig.runtime
                .resolve_workflow_notification_source(owner, "guest", "another-room", &source)
                .is_err(),
            "a source of another room is not this room's"
        );
        assert!(rig
            .runtime
            .resolve_workflow_notification_source(owner, owner, "another-room", &source)
            .is_ok());
    }
}
