//! Who answers for a workflow: who may name them, and the kernel-kept trigger that carries a
//! run's needs to a responsible agent as an ordinary prompt of its own Room. The trigger lives
//! on the agent's kernel and follows the workflow's "run finished" source like any other
//! notification trigger, whichever kernel holds the workflow.
use super::agent_trigger_notification::notification_subscription_id;
use super::workflow_request_runtime_state::WorkflowRequestOutcome;
use super::*;
use crate::agent::AgentInstance;
use crate::durable_state::workflow_notifications::{NotificationOperation, NotificationOutcome};
use crate::local::{
    SetWorkflowResponsibleRequest, WorkflowNotificationSource, WorkflowNotificationSubscription,
    WorkflowNotificationTargetKind, WorkflowResponsibleTarget,
};
use crate::runtime::room_tool_admission::denied;
use crate::session::{
    AgentTrigger, AgentTriggerNotificationSource, AgentTriggerSource, ReplacedListener,
    ResponsibleForWorkflow, ResponsibleNotice, ResponsibleNoticeKind, RuntimeSession,
    WorkflowDefinition, WorkflowResponsible,
};
use crate::transport::relay_peer::{
    PeerAgentAddress, RelayPeerRequest, RelayPeerResponse, ResponsibleTriggerAction,
    RELAY_PEER_PROTOCOL_VERSION,
};

/// The part of a responsible agent's prompt that is the same for every run: the kernel's own
/// words, never text a run produced.
pub(super) const RESPONSIBLE_PROMPT_TRAILER: &str = "You are responsible for this workflow. Fix what you can and run it again with `workflow run`; if you need the person, ask from this Room.";

/// A responsible agent's subscription is told apart by its id, so its kernel's end of the
/// subscription is recognized when the workflow's kernel hears it dropped.
pub(super) const RESPONSIBLE_SUBSCRIPTION_PREFIX: &str = "wf_responsible_";

fn agent_label(agent: &AgentInstance) -> String {
    agent.alias().unwrap_or(agent.agent_ref()).to_owned()
}

fn unavailable() -> DaemonError {
    denied(
        "that agent is not available as responsible; pick an ordinary agent of one of your Rooms",
    )
}

/// An agent the way a request names it, with its kernel.
struct NamedAgent<'a> {
    kernel_id: &'a str,
    room_id: &'a str,
    agent_id: &'a str,
}

fn named(target: &WorkflowResponsibleTarget) -> Option<NamedAgent<'_>> {
    match target {
        WorkflowResponsibleTarget::Person => None,
        WorkflowResponsibleTarget::Agent {
            kernel_id,
            room_id,
            agent_id,
        } => Some(NamedAgent {
            kernel_id,
            room_id,
            agent_id,
        }),
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn home_kernel_id(&self) -> String {
        self.config_projection.snapshot().daemon_id
    }

    /// The person may always set; an agent may take a workflow for itself, and may hand it to
    /// anyone only while it is the current responsible or the workflow's creator (or the
    /// creator's ancestor). A guest of the Room may not set.
    pub(super) fn ensure_may_set_responsible(
        &self,
        room: &RuntimeSession,
        workflow: &WorkflowDefinition,
        caller_user_id: &str,
        caller_agent_id: Option<&str>,
        target: &WorkflowResponsibleTarget,
    ) -> Result<(), DaemonError> {
        let guest =
            || denied("only the owner of this Room can change who is responsible for a workflow");
        let Some(caller_agent_id) = caller_agent_id else {
            return if self.acts_for_owner_in(
                room.owner_user_id(),
                caller_user_id,
                room.owner_user_id(),
            ) {
                Ok(())
            } else {
                Err(guest())
            };
        };
        let actor = self.agent_store.get_agent(caller_agent_id)?;
        if self.drives_as_room_guest(room, &actor) {
            return Err(guest());
        }
        let home = self.home_kernel_id();
        let actor_is = |kernel: &str, agent: &str| kernel == home && agent == actor.id();
        if named(target).is_some_and(|agent| actor_is(agent.kernel_id, agent.agent_id))
            || workflow
                .responsible()
                .agent()
                .is_some_and(|agent| actor_is(agent.kernel_id, agent.agent_id))
        {
            return Ok(());
        }
        if crate::runtime::room_tool_admission::owns_object(
            &actor,
            workflow.created_by_agent_id(),
            &self.agent_store.get_session_agents(room.id()),
        ) {
            Ok(())
        } else {
            Err(denied(
                "only the person, the current responsible or the creator of this workflow can name who is responsible; take it for yourself with `workflow responsible <workflow> me`",
            ))
        }
    }

    /// Makes sure the workflow's "run finished" source exists for the Room's owner, and returns it.
    pub(super) fn ensure_run_finished_source(
        &self,
        room: &RuntimeSession,
        workflow: &WorkflowDefinition,
    ) -> Result<WorkflowNotificationSource, DaemonError> {
        let source = super::run_finished_notifications::run_finished_source(room, workflow);
        match self
            .durable_state_store
            .notify(NotificationOperation::Register(
                crate::durable_state::workflow_notifications::SourceAdmission {
                    source,
                    workflow_json: String::new(),
                },
            ))? {
            NotificationOutcome::Source(source) => Ok(source),
            _ => Err(crate::durable_state::workflow_notifications::error(
                "unexpected notification registration result",
            )),
        }
    }
}

impl KernelRuntimeState {
    pub(super) async fn execute_workflow_set_responsible_request(
        &self,
        request: SetWorkflowResponsibleRequest,
        caller_user_id: &str,
        caller_agent_id: Option<&str>,
    ) -> WorkflowRequestOutcome {
        match self
            .set_workflow_responsible(request, caller_user_id, caller_agent_id)
            .await
        {
            Ok((workflow, session)) => (
                Ok(LocalDaemonResponse::WorkflowResponsibleSet {
                    workflow,
                    session: session.clone(),
                }),
                Some(session),
            ),
            Err(error) => (Err(error), None),
        }
    }

    /// The person opened the run that needed them: its attention closes, and the run leaves
    /// the Room's snapshot with the next save.
    pub(super) fn execute_workflow_clear_run_attention_request(
        &self,
        request: crate::local::ClearWorkflowRunAttentionRequest,
    ) -> WorkflowRequestOutcome {
        let result = (|| {
            let cleared = self
                .owned
                .session_store
                .write()
                .clear_workflow_run_attention(&request.session_id, &request.run_ref);
            let run = match cleared {
                Ok(run) => run,
                // A run that already left the snapshot has had its attention closed.
                Err(DaemonError::WorkflowRunNotFound { .. }) => {
                    let room = self.owned.session_store.get_session(&request.session_id)?;
                    self.owned
                        .durable_state_store
                        .resolve_workflow_run(room.host_daemon_id(), room.id(), &request.run_ref)?
                        .ok_or_else(|| DaemonError::WorkflowRunNotFound {
                            session_id: request.session_id.clone(),
                            workflow_run_id: request.run_ref.clone(),
                        })?
                }
                Err(error) => return Err(error),
            };
            let session = self.owned.persist_workflow_runtime_session(
                &request.session_id,
                "workflow_run_attention_cleared",
            )?;
            Ok((run, session))
        })();
        match result {
            Ok((run, session)) => (
                Ok(LocalDaemonResponse::WorkflowRunAttentionCleared {
                    run,
                    session: session.clone(),
                }),
                Some(session),
            ),
            Err(error) => (Err(error), None),
        }
    }

    /// The Room a request that can create a workflow names, and the workflows it holds before
    /// the request runs.
    pub(super) fn workflow_ids_in_request_room(
        &self,
        request: &LocalDaemonRequest,
    ) -> Option<(String, BTreeSet<String>)> {
        if !matches!(
            request,
            LocalDaemonRequest::CreateWorkflow(_)
                | LocalDaemonRequest::ApplyWorkflowCode(_)
                | LocalDaemonRequest::ApplyWorkflowCodeArtifact(_)
                | LocalDaemonRequest::RunWorkflowCode(_)
                | LocalDaemonRequest::RunWorkflowCodeArtifact(_)
                | LocalDaemonRequest::LoadWorkflowRegistryEntry(_)
                | LocalDaemonRequest::RunWorkflowRegistryEntry(_)
        ) {
            return None;
        }
        let Some(crate::runtime::session_membership::scope::SessionMembershipScope::SessionId(
            room_id,
        )) = crate::runtime::session_membership::scope::request_session_scope(request)
        else {
            return None;
        };
        let room = self.owned.session_store.get_session(&room_id).ok()?;
        let known = room
            .workflows()
            .iter()
            .map(|workflow| workflow.id().to_owned())
            .collect();
        Some((room_id, known))
    }

    /// Every workflow `agent_id` created in `room_id` since `known` starts with that agent as
    /// its responsible. A refusal leaves the person responsible: the workflow still works.
    /// Answers whether any workflow changed.
    pub(super) async fn make_creator_responsible(
        &self,
        room_id: &str,
        agent_id: &str,
        known: &BTreeSet<String>,
    ) -> bool {
        let Ok(room) = self.owned.session_store.get_session(room_id) else {
            return false;
        };
        // A sub-agent ends and disappears: a workflow it made starts with the person.
        let Some(agent) = self
            .owned
            .agent_store
            .get_agent(agent_id)
            .ok()
            .filter(|agent| agent.parent_agent_id().is_none())
        else {
            return false;
        };
        let target = WorkflowResponsibleTarget::Agent {
            kernel_id: self.owned.home_kernel_id(),
            room_id: room_id.to_owned(),
            agent_id: agent.id().to_owned(),
        };
        let mut changed = false;
        for workflow in room.workflows().iter().filter(|workflow| {
            !known.contains(workflow.id()) && workflow.created_by_agent_id() == Some(agent_id)
        }) {
            match self
                .change_workflow_responsible(&room, workflow, target.clone(), None, true)
                .await
            {
                Ok(_) => changed = true,
                Err(error) => {
                    tracing::warn!(%error, workflow_id = workflow.id(), "creating agent was not made responsible")
                }
            }
        }
        changed
    }

    async fn set_workflow_responsible(
        &self,
        request: SetWorkflowResponsibleRequest,
        caller_user_id: &str,
        caller_agent_id: Option<&str>,
    ) -> Result<(WorkflowDefinition, RuntimeSession), DaemonError> {
        self.authorize_current_external_command()?;
        let room = self.owned.session_store.get_session(&request.session_id)?;
        let workflow = self
            .owned
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        self.owned.ensure_may_set_responsible(
            &room,
            &workflow,
            caller_user_id,
            caller_agent_id,
            &request.responsible,
        )?;
        let workflow = self
            .change_workflow_responsible(&room, &workflow, request.responsible, None, true)
            .await?;
        let session = self.owned.session_snapshot(&request.session_id)?;
        Ok((workflow, session))
    }

    /// Replaces the responsible. The new agent's trigger exists before the field changes, so a
    /// refused attach leaves everything as it was; the old agent's trigger goes after, unless
    /// `retire_previous` is false (its agent or machine is gone: there is nobody to tell).
    pub(super) async fn change_workflow_responsible(
        &self,
        room: &RuntimeSession,
        workflow: &WorkflowDefinition,
        target: WorkflowResponsibleTarget,
        notice: Option<ResponsibleNotice>,
        retire_previous: bool,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let previous = workflow.responsible().clone();
        let responsible = match &target {
            WorkflowResponsibleTarget::Person => WorkflowResponsible::Person,
            WorkflowResponsibleTarget::Agent {
                kernel_id,
                room_id,
                agent_id,
            } => WorkflowResponsible::Agent {
                kernel_id: kernel_id.clone(),
                room_id: room_id.clone(),
                agent_id: agent_id.clone(),
                label: self
                    .attach_responsible_trigger(room, workflow, &target)
                    .await?,
            },
        };
        let updated = self.owned.session_store.write().set_workflow_responsible(
            room.id(),
            workflow.id(),
            responsible.clone(),
            notice,
        )?;
        if retire_previous && previous.agent().is_some() && previous != responsible {
            if let Err(error) = self
                .detach_responsible_trigger(room, workflow, &previous)
                .await
            {
                tracing::warn!(%error, workflow_id = workflow.id(), "former responsible's trigger was not removed");
            }
        }
        Ok(updated)
    }

    /// Creates, on the responsible agent's kernel, the trigger that queues a prompt of the agent
    /// for every occurrence of the workflow's "run finished" source. Answers the agent's name.
    async fn attach_responsible_trigger(
        &self,
        room: &RuntimeSession,
        workflow: &WorkflowDefinition,
        target: &WorkflowResponsibleTarget,
    ) -> Result<String, DaemonError> {
        let Some(agent) = named(target) else {
            return Err(unavailable());
        };
        let source = self.owned.ensure_run_finished_source(room, workflow)?;
        let for_workflow = ResponsibleForWorkflow {
            kernel_id: room.host_daemon_id().into(),
            room_id: room.id().into(),
            workflow_id: workflow.id().into(),
        };
        if agent.kernel_id == self.owned.home_kernel_id() {
            return self
                .install_responsible_trigger(
                    for_workflow,
                    &source,
                    room.owner_user_id(),
                    agent.room_id,
                    agent.agent_id,
                )
                .await;
        }
        match self
            .peer_agent_link(
                room.owner_user_id(),
                agent.kernel_id,
                RelayPeerRequest::ApplyWorkflowResponsible {
                    protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                    action: ResponsibleTriggerAction::Attach,
                    workflow: for_workflow,
                    source,
                    agent: PeerAgentAddress {
                        room: agent.room_id.into(),
                        agent: agent.agent_id.into(),
                    },
                },
            )
            .await?
        {
            RelayPeerResponse::WorkflowResponsibleApplied { label, .. } => Ok(label),
            _ => Err(unavailable()),
        }
    }

    async fn detach_responsible_trigger(
        &self,
        room: &RuntimeSession,
        workflow: &WorkflowDefinition,
        previous: &WorkflowResponsible,
    ) -> Result<(), DaemonError> {
        let Some(agent) = previous.agent() else {
            return Ok(());
        };
        let for_workflow = ResponsibleForWorkflow {
            kernel_id: room.host_daemon_id().into(),
            room_id: room.id().into(),
            workflow_id: workflow.id().into(),
        };
        if agent.kernel_id == self.owned.home_kernel_id() {
            return self
                .remove_responsible_triggers_of(agent.room_id, agent.agent_id, &for_workflow)
                .await;
        }
        let source = super::run_finished_notifications::run_finished_source(room, workflow);
        self.peer_agent_link(
            room.owner_user_id(),
            agent.kernel_id,
            RelayPeerRequest::ApplyWorkflowResponsible {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                action: ResponsibleTriggerAction::Detach,
                workflow: for_workflow,
                source,
                agent: PeerAgentAddress {
                    room: agent.room_id.into(),
                    agent: agent.agent_id.into(),
                },
            },
        )
        .await
        .map(|_| ())
    }

    /// An ordinary top-level agent of a live Room of `owner`, and not a guest's.
    fn responsible_candidate(
        &self,
        owner: &str,
        room_id: &str,
        agent: &str,
    ) -> Result<(AgentInstance, RuntimeSession), DaemonError> {
        let agent = self
            .resolve_peer_agent(owner, room_id, agent)
            .map_err(|_| unavailable())?;
        let room = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .map_err(|_| unavailable())?;
        if room.status() == crate::session::SessionStatus::Ended
            || self.drives_as_room_guest(&room, &agent)
        {
            return Err(unavailable());
        }
        Ok((agent, room))
    }

    /// Creates the trigger and its subscription in the agent's own Room, once per workflow.
    /// Answers the agent's name.
    pub(super) async fn install_responsible_trigger(
        &self,
        workflow: ResponsibleForWorkflow,
        source: &WorkflowNotificationSource,
        owner: &str,
        agent_room_id: &str,
        agent_ref: &str,
    ) -> Result<String, DaemonError> {
        let (agent, agent_room) = self.responsible_candidate(owner, agent_room_id, agent_ref)?;
        let label = agent_label(&agent);
        if agent_room.agent_triggers().iter().any(|trigger| {
            trigger.agent_id() == agent.id() && trigger.responsible_for() == Some(&workflow)
        }) {
            return Ok(label);
        }
        let own: Vec<AgentTrigger> = agent_room
            .agent_triggers()
            .iter()
            .filter(|trigger| {
                trigger.agent_id() == agent.id()
                    && trigger.responsible_for().is_none()
                    && matches!(
                        trigger.source(),
                        AgentTriggerSource::Notification {
                            source: AgentTriggerNotificationSource::Workflow { source_id },
                            ..
                        } if *source_id == source.source_id
                    )
            })
            .cloned()
            .collect();
        let subscription_id = format!(
            "{RESPONSIBLE_SUBSCRIPTION_PREFIX}{:032x}",
            rand::random::<u128>()
        );
        let trigger = self
            .owned
            .session_store
            .write()
            .create_responsible_trigger(
                agent_room.id(),
                agent.id(),
                owner,
                source.source_id.clone(),
                subscription_id.clone(),
                source.display_name.clone(),
                workflow,
                RESPONSIBLE_PROMPT_TRAILER.into(),
                own.first().map(AgentTrigger::listener),
            )?;
        let subscription = self.agent_listener_subscription(
            subscription_id,
            &source.source_id,
            &source.kernel_id,
            owner,
            agent_room.id(),
            &trigger,
        );
        let attached = self
            .subscribe_agent_notification(&self.owned.home_kernel_id(), subscription)
            .await;
        // The agent hears each run once: the listener it had on this source gives way, so no
        // delivery is left for routing to skip for ever.
        let attached = match attached {
            Ok(()) => {
                for own in &own {
                    self.remove_listener_trigger(agent_room.id(), own).await;
                }
                self.owned
                    .persist_agent_trigger_session(agent_room.id(), "agent_trigger_created")
                    .map(|_| ())
            }
            error => error,
        };
        if let Err(error) = attached {
            self.remove_listener_trigger(agent_room.id(), &trigger)
                .await;
            return Err(error);
        }
        Ok(label)
    }

    /// Deletes the agent's responsible trigger of `workflow` and ends its subscription.
    pub(super) async fn remove_responsible_triggers_of(
        &self,
        agent_room_id: &str,
        agent_id: &str,
        workflow: &ResponsibleForWorkflow,
    ) -> Result<(), DaemonError> {
        let Ok(agent_room) = self.owned.session_store.get_session(agent_room_id) else {
            return Ok(());
        };
        let triggers: Vec<AgentTrigger> = agent_room
            .agent_triggers()
            .iter()
            .filter(|trigger| {
                trigger.agent_id() == agent_id && trigger.responsible_for() == Some(workflow)
            })
            .cloned()
            .collect();
        if triggers.is_empty() {
            return Ok(());
        }
        // The agent is still there: it gets back the listener it had, so it keeps hearing the
        // workflow's runs by its own choice.
        let agent_is_live = self
            .owned
            .agent_store
            .get_agent(agent_id)
            .is_ok_and(|agent| !agent.archived());
        for trigger in triggers {
            let subscription = notification_subscription_id(&trigger).and_then(|id| {
                self.agent_notification_subscriptions()
                    .into_iter()
                    .find(|s| s.subscription_id == id)
            });
            self.remove_listener_trigger(agent_room_id, &trigger).await;
            if let (true, Some(listener), Some(subscription)) =
                (agent_is_live, trigger.replaced_listener(), subscription)
            {
                self.restore_own_listener(agent_room_id, &trigger, listener, &subscription)
                    .await?;
            }
        }
        self.owned
            .persist_agent_trigger_session(agent_room_id, "agent_trigger_deleted")?;
        Ok(())
    }

    /// The agent's own listener on the source its responsible trigger followed, as it was set.
    async fn restore_own_listener(
        &self,
        agent_room_id: &str,
        responsible: &AgentTrigger,
        listener: &ReplacedListener,
        responsible_subscription: &WorkflowNotificationSubscription,
    ) -> Result<(), DaemonError> {
        let AgentTriggerSource::Notification {
            source: source @ AgentTriggerNotificationSource::Workflow { source_id },
            display_name,
            ..
        } = responsible.source()
        else {
            return Ok(());
        };
        let owner = &responsible_subscription.owner_user_id;
        let subscription_id = format!("wf_subscription_{:032x}", rand::random::<u128>());
        let trigger = {
            let mut store = self.owned.session_store.write();
            let trigger = store.create_agent_notification_trigger(
                agent_room_id,
                responsible.agent_id(),
                owner,
                source.clone(),
                subscription_id.clone(),
                display_name.clone(),
                Some(listener.prompt().to_owned()),
            )?;
            store.update_agent_trigger(
                agent_room_id,
                trigger.id(),
                None,
                None,
                Some(listener.enabled()),
            )?
        };
        let subscription = self.agent_listener_subscription(
            subscription_id,
            source_id,
            &responsible_subscription.source_kernel_id,
            owner,
            agent_room_id,
            &trigger,
        );
        if let Err(error) = self
            .subscribe_agent_notification(&self.owned.home_kernel_id(), subscription)
            .await
        {
            self.remove_listener_trigger(agent_room_id, &trigger).await;
            return Err(error);
        }
        Ok(())
    }

    /// The subscription that carries `source`'s occurrences to the agent of `trigger`.
    fn agent_listener_subscription(
        &self,
        subscription_id: String,
        source_id: &str,
        source_kernel_id: &str,
        owner: &str,
        agent_room_id: &str,
        trigger: &AgentTrigger,
    ) -> WorkflowNotificationSubscription {
        WorkflowNotificationSubscription {
            subscription_id,
            source_id: source_id.into(),
            owner_user_id: owner.into(),
            created_by_user_id: owner.into(),
            target_kernel_id: self.owned.home_kernel_id(),
            target_kind: WorkflowNotificationTargetKind::Agent,
            session_id: agent_room_id.into(),
            workflow_id: String::new(),
            publication_id: trigger.id().into(),
            endpoint_id: trigger.agent_id().into(),
            queue_id: "agent".into(),
            source_available: true,
            source_kernel_id: source_kernel_id.into(),
            agent_id: Some(trigger.agent_id().into()),
            trigger_id: Some(trigger.id().into()),
        }
    }

    /// Deletes a notification trigger the kernel manages and ends its subscription.
    async fn remove_listener_trigger(&self, agent_room_id: &str, trigger: &AgentTrigger) {
        let _ = self
            .owned
            .session_store
            .write()
            .delete_agent_trigger(agent_room_id, trigger.id());
        self.detach_deleted_notification_trigger(trigger).await;
    }

    /// The responsible agent was deleted: the workflows it answered for are the person's again.
    pub(super) async fn fall_back_for_deleted_agent(&self, agent: &AgentInstance) {
        let home = self.owned.home_kernel_id();
        for room in self.owned.session_store.list_sessions() {
            for workflow in room.workflows().iter().filter(|workflow| {
                workflow.responsible().agent().is_some_and(|responsible| {
                    responsible.kernel_id == home && responsible.agent_id == agent.id()
                })
            }) {
                self.fall_back_to_person(&room, workflow, ResponsibleNoticeKind::AgentGone)
                    .await;
            }
        }
    }

    /// The responsible's kernel was removed from the owner's account: the workflows its agents
    /// answered for are the person's again. A kernel that is merely offline keeps its workflows.
    pub(super) async fn fall_back_for_gone_kernels(&self) {
        if !self
            .owned
            .notification_inventory_projection
            .has_directory_history()
        {
            return;
        }
        let home = self.owned.home_kernel_id();
        let rooms = self.owned.session_store.list_sessions();
        let responsible_kernels: BTreeSet<String> = rooms
            .iter()
            .flat_map(|room| room.workflows())
            .filter_map(|workflow| workflow.responsible().agent())
            .map(|agent| agent.kernel_id.to_owned())
            .filter(|kernel| *kernel != home)
            .collect();
        if responsible_kernels.is_empty() {
            return;
        }
        let gone = self
            .owned
            .notification_inventory_projection
            .gone_kernels(&responsible_kernels);
        for room in &rooms {
            for workflow in room.workflows().iter().filter(|workflow| {
                workflow
                    .responsible()
                    .agent()
                    .is_some_and(|agent| gone.contains(agent.kernel_id))
            }) {
                self.fall_back_to_person(room, workflow, ResponsibleNoticeKind::MachineGone)
                    .await;
            }
        }
    }

    /// The person answers again for `workflow`, with a notice that says why.
    pub(super) async fn fall_back_to_person(
        &self,
        room: &RuntimeSession,
        workflow: &WorkflowDefinition,
        kind: ResponsibleNoticeKind,
    ) {
        let Some(label) = workflow
            .responsible()
            .agent()
            .map(|responsible| responsible.label.to_owned())
        else {
            return;
        };
        let changed = self
            .change_workflow_responsible(
                room,
                workflow,
                WorkflowResponsibleTarget::Person,
                Some(ResponsibleNotice { kind, label }),
                false,
            )
            .await;
        match changed {
            Ok(_) => {
                if let Ok(session) = self.owned.session_snapshot(room.id()) {
                    if let Err(error) = self
                        .append_session_durable_event("session.updated", &session, "workflow")
                        .await
                    {
                        tracing::warn!(%error, "responsible fallback was not recorded");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, workflow_id = workflow.id(), "responsible fallback failed")
            }
        }
    }

    /// The responsible's kernel dropped the subscription its agent held on a source of a
    /// workflow here: when that subscription is the current responsible's, its agent is gone
    /// and the person answers again.
    pub(super) async fn fall_back_for_dropped_responsible_subscription(
        &self,
        subscription: &WorkflowNotificationSubscription,
    ) {
        if !subscription
            .subscription_id
            .starts_with(RESPONSIBLE_SUBSCRIPTION_PREFIX)
        {
            return;
        }
        let Ok((sources, _, _)) = self
            .owned
            .durable_state_store
            .notification_inventory(&subscription.owner_user_id)
        else {
            return;
        };
        let Some(source) = sources
            .iter()
            .find(|source| source.source_id == subscription.source_id)
        else {
            return;
        };
        let Ok(room) = self.owned.session_store.get_session(&source.session_id) else {
            return;
        };
        let Some(workflow) = room.workflow(&source.workflow_id) else {
            return;
        };
        let theirs = workflow.responsible().agent().is_some_and(|agent| {
            agent.kernel_id == subscription.target_kernel_id
                && agent.agent_id == subscription.endpoint_id
        });
        if theirs {
            self.fall_back_to_person(&room, workflow, ResponsibleNoticeKind::AgentGone)
                .await;
        }
    }

    /// Reads, or changes, who answers for a workflow of another Room of the owner, on this
    /// kernel or on `kernel`, as `caller`, an agent of this kernel. No `responsible` reads.
    pub(crate) async fn workflow_responsible_in(
        &self,
        caller: &AgentInstance,
        room: &str,
        kernel: Option<&str>,
        workflow_ref: &str,
        responsible: Option<WorkflowResponsibleTarget>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        self.authorize_current_external_command()?;
        let Some(kernel) = kernel else {
            let caller_room = self.owned.session_store.get_session(caller.session_id())?;
            let target_room = self
                .peer_room(caller_room.owner_user_id(), room)
                .ok()
                .filter(|target| {
                    target.id() == caller_room.id()
                        || !self.drives_as_room_guest(&caller_room, caller)
                })
                .ok_or_else(|| denied("that Room is not available: pick one of your own Rooms"))?;
            let Some(responsible) = responsible else {
                return self
                    .owned
                    .session_store
                    .read()
                    .resolve_workflow_ref(target_room.id(), workflow_ref);
            };
            let (workflow, session) = self
                .set_workflow_responsible(
                    SetWorkflowResponsibleRequest {
                        session_id: target_room.id().into(),
                        workflow_ref: workflow_ref.into(),
                        responsible,
                    },
                    caller.owner_user_id(),
                    Some(caller.id()),
                )
                .await?;
            // The request path saves the Room after its change; this path saves it the same way.
            self.append_session_durable_event("session.updated", &session, "workflow")
                .await?;
            return Ok(workflow);
        };
        let (owner, sender) = self.peer_caller_of_agent(caller, kernel)?;
        match self
            .peer_agent_link(
                &owner,
                kernel,
                RelayPeerRequest::SetWorkflowResponsible {
                    protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                    sender,
                    room: room.into(),
                    workflow_ref: workflow_ref.into(),
                    responsible,
                },
            )
            .await?
        {
            RelayPeerResponse::WorkflowResponsibleChanged { workflow, .. } => Ok(workflow),
            _ => Err(unavailable()),
        }
    }

    /// A request from another kernel of the owner: its agent takes or hands over a workflow of
    /// a Room here, or this kernel keeps or drops the trigger of one of its agents.
    pub(super) async fn receive_workflow_responsible_request(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        match request {
            RelayPeerRequest::SetWorkflowResponsible {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                sender,
                room,
                workflow_ref,
                responsible,
            } => {
                let room = self.peer_room(owner, &room).map_err(|_| unavailable())?;
                let workflow = self
                    .owned
                    .session_store
                    .read()
                    .resolve_workflow_ref(room.id(), &workflow_ref)?;
                let Some(responsible) = responsible else {
                    return Ok(RelayPeerResponse::WorkflowResponsibleChanged {
                        protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                        workflow,
                    });
                };
                // The sender's own kernel vouched for it; here only what it may do is decided.
                let is_sender = |kernel_id: &str, agent_id: &str| {
                    kernel_id == sender.kernel_id && agent_id == sender.agent_id
                };
                if !(named(&responsible)
                    .is_some_and(|agent| is_sender(agent.kernel_id, agent.agent_id))
                    || workflow
                        .responsible()
                        .agent()
                        .is_some_and(|agent| is_sender(agent.kernel_id, agent.agent_id)))
                {
                    return Err(denied(
                        "only the current responsible can hand this workflow over; take it for yourself with `workflow responsible <workflow> me`",
                    ));
                }
                let workflow = self
                    .change_workflow_responsible(&room, &workflow, responsible, None, true)
                    .await?;
                let session = self.owned.session_snapshot(room.id())?;
                self.append_session_durable_event("session.updated", &session, "workflow")
                    .await?;
                Ok(RelayPeerResponse::WorkflowResponsibleChanged {
                    protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                    workflow,
                })
            }
            RelayPeerRequest::ApplyWorkflowResponsible {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                action,
                workflow,
                source,
                agent,
            } => {
                if workflow.kernel_id != kernel
                    || source.kernel_id != kernel
                    || source.owner_user_id != owner
                    || source.session_id != workflow.room_id
                    || source.workflow_id != workflow.workflow_id
                {
                    return Err(unavailable());
                }
                match action {
                    ResponsibleTriggerAction::Attach => {
                        let label = self
                            .install_responsible_trigger(
                                workflow,
                                &source,
                                owner,
                                &agent.room,
                                &agent.agent,
                            )
                            .await?;
                        Ok(RelayPeerResponse::WorkflowResponsibleApplied {
                            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                            label,
                        })
                    }
                    ResponsibleTriggerAction::Detach => {
                        if let Ok(target) =
                            self.resolve_peer_agent(owner, &agent.room, &agent.agent)
                        {
                            self.remove_responsible_triggers_of(
                                target.session_id(),
                                target.id(),
                                &workflow,
                            )
                            .await?;
                        }
                        Ok(RelayPeerResponse::WorkflowResponsibleApplied {
                            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                            label: String::new(),
                        })
                    }
                }
            }
            _ => Err(unavailable()),
        }
    }
}
