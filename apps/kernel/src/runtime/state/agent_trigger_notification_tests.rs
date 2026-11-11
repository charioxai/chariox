use super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt;
use super::workflow_notification_peers::VERSION;
use super::*;
use crate::local::{
    AgentTriggerSourceRequest, DeleteAgentTriggerRequest, DetachWorkflowNotificationRequest,
    ListWorkflowNotificationsRequest, LocalDaemonRequest,
    RegisterWorkflowNotificationSourceRequest, UpdateAgentTriggerRequest,
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
    attachment_id: String,
}

async fn rig() -> Rig {
    let (root, runtime, session_id, agent_id, attachment_id, _, _) =
        runtime_with_active_prompt().await;
    Rig {
        _root: root,
        runtime,
        session_id,
        agent_id,
        attachment_id,
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
        let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = response else {
            panic!()
        };
        source.source_id
    }

    fn complete(&self, workflow: &str, run_id: &str, message: &str) {
        self.complete_with_invocation(workflow, run_id, message, None);
    }

    fn complete_with_invocation(
        &self,
        workflow: &str,
        run_id: &str,
        message: &str,
        invocation: Option<crate::session::WorkflowPublicationInvocationEnvelope>,
    ) {
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
            invocation,
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

    async fn trigger_on(&self, source_id: &str) -> String {
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = self
            .runtime
            .create_agent_trigger(self.request(source_id), "local")
            .await
            .unwrap()
        else {
            panic!()
        };
        trigger.id().into()
    }

    fn trigger_listening(&self, trigger_id: &str) -> bool {
        trigger_listening_in(
            &self
                .runtime
                .owned
                .session_snapshot(&self.session_id)
                .unwrap(),
            trigger_id,
        )
    }

    async fn set_source_enabled(&self, workflow: &str, enabled: bool) {
        let (result, _) = self
            .runtime
            .execute_workflow_request(
                LocalDaemonRequest::RegisterWorkflowNotificationSource(
                    RegisterWorkflowNotificationSourceRequest {
                        session_id: self.session_id.clone(),
                        workflow_ref: workflow.into(),
                        exit_node_id: "exit-1".into(),
                        enabled,
                    },
                ),
                "local".into(),
                None,
            )
            .await;
        result.unwrap();
    }
}

fn trigger_listening_in(session: &crate::session::RuntimeSession, trigger_id: &str) -> bool {
    match session
        .agent_triggers()
        .iter()
        .find(|t| t.id() == trigger_id)
        .unwrap()
        .source()
    {
        AgentTriggerSource::Notification {
            source_available, ..
        } => *source_available,
        other => panic!("not a notification trigger: {other:?}"),
    }
}

// MP-08 / MP-10: the trigger tells every client whether its source's kernel is reachable.
#[tokio::test]
async fn a_notification_trigger_follows_the_availability_of_its_remote_source() {
    let rig = rig().await;
    let source_id = "wf_source_remote";
    let trigger_id = rig
        .runtime
        .owned
        .session_store
        .write()
        .create_agent_notification_trigger(
            &rig.session_id,
            &rig.agent_id,
            "local",
            AgentTriggerNotificationSource::Workflow {
                source_id: source_id.into(),
            },
            "sub-remote".into(),
            "Studio pipeline".into(),
            None,
        )
        .unwrap()
        .id()
        .to_owned();
    let available_in = |session: &crate::session::RuntimeSession| {
        let trigger = session
            .agent_triggers()
            .iter()
            .find(|t| t.id() == trigger_id)
            .unwrap();
        match trigger.source() {
            AgentTriggerSource::Notification {
                source_available, ..
            } => *source_available,
            other => panic!("not a notification trigger: {other:?}"),
        }
    };
    let cache_available = || {
        rig.runtime
            .owned
            .durable_state_store
            .notify(NotificationOperation::Cache {
                owner: "local".into(),
                kernel: "other-home".into(),
                sources: vec![crate::local::WorkflowNotificationSourceSummary {
                    kind: crate::local::NotificationSourceKind::WorkflowExit,
                    source_id: source_id.into(),
                    kernel_id: "other-home".into(),
                    session_id: "other-session".into(),
                    workflow_id: "other-workflow".into(),
                    exit_node_id: "exit".into(),
                    exit_ordinal: 1,
                    name: "Studio pipeline".into(),
                    display_name: "Studio pipeline".into(),
                    available: true,
                }],
            })
            .unwrap();
        rig.runtime
            .sync_agent_notification_trigger_availability()
            .unwrap();
        available_in(&rig.runtime.owned.session_snapshot(&rig.session_id).unwrap())
    };
    assert!(
        cache_available(),
        "an available source leaves the trigger available"
    );
    // The inventory refresh that finds no source kernel marks every cached source offline.
    rig.runtime
        .refresh_notification_sources_from_inventory("local", false)
        .await
        .unwrap();
    assert!(
        !available_in(&rig.runtime.owned.session_snapshot(&rig.session_id).unwrap()),
        "a source whose kernel is offline marks the trigger unavailable"
    );
    assert!(cache_available(), "the trigger recovers with its source");
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
        assert!(text.contains("\"display_name\":\"pipeline\""), "{text}");
    }
    let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    assert_eq!(session.agent_triggers()[0].fired(), 3);
    assert_eq!(session.agent_triggers()[0].coalesced(), 0);
    // Settled receipts are not delivered twice.
    rig.pass().await;
    assert_eq!(rig.queued_prompts().len(), 3);
}

// MP-08 / MP-09 / MP-10 / MP-11: subscribing one agent to one source again is the same listener.
#[tokio::test]
async fn subscribing_the_same_agent_to_a_source_twice_keeps_one_listener() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);

    let LocalDaemonResponse::AgentTriggerCreated { trigger: first, .. } = rig
        .runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    let mut again = rig.request(&source);
    again.prompt = Some("Summarise the pipeline result".into());
    let LocalDaemonResponse::AgentTriggerCreated {
        trigger: second, ..
    } = rig
        .runtime
        .create_agent_trigger(again, "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(second.id(), first.id());
    assert_eq!(second.prompt(), "Summarise the pipeline result");
    let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    assert_eq!(session.agent_triggers().len(), 1);
    assert_eq!(rig.subscriptions().len(), 1);

    rig.complete(&workflow, "run-0", "release 0 approved");
    rig.pass().await;
    let queued = rig.queued_prompts();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(queued[0].starts_with("Summarise the pipeline result"));
}

// MP-08 / MP-09 / MP-10 / MP-11: a store that already holds two listeners of one agent and
// source delivers each occurrence once.
#[tokio::test]
async fn listeners_already_duplicated_in_a_store_deliver_each_occurrence_once() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    let home = rig.runtime.owned.config_projection.snapshot().daemon_id;
    let mut copy = rig.subscriptions().remove(0);
    copy.subscription_id = "wf_subscription_00000000000000000000000000000001".into();
    let trigger = rig
        .runtime
        .owned
        .session_store
        .write()
        .create_agent_notification_trigger(
            &rig.session_id,
            &rig.agent_id,
            "local",
            AgentTriggerNotificationSource::Workflow {
                source_id: source.clone(),
            },
            copy.subscription_id.clone(),
            "pipeline".into(),
            Some("Triage what the pipeline reports".into()),
        )
        .unwrap();
    copy.publication_id = trigger.id().into();
    copy.trigger_id = Some(trigger.id().into());
    rig.runtime
        .subscribe_agent_notification(&home, copy)
        .await
        .unwrap();
    assert_eq!(rig.subscriptions().len(), 2);

    rig.complete(&workflow, "run-0", "release 0 approved");
    rig.pass().await;
    let queued = rig.queued_prompts();
    assert_eq!(queued.len(), 1, "{queued:?}");
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
                LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                    session_id: rig.session_id.clone(),
                }),
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

/// Enrolls the rig's kernel, whose room was made before under `local`, for `kernel-owner`.
fn enroll(rig: &Rig) -> &'static str {
    let mut config = rig.runtime.owned.config_projection.snapshot();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    rig.runtime.owned.config_projection.update(config);
    "kernel-owner"
}

// MP-08 / MP-09 / MP-10 / MP-11: the enrolled owner of a room made before enrollment picks from
// all its sources, as the owner does; a collaborator still sees this room's only.
#[tokio::test]
async fn the_enrolled_owner_resolves_its_sources_of_any_room() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    let enrolled = enroll(&rig);
    let resolved = rig.runtime.resolve_workflow_notification_source(
        "local",
        enrolled,
        "another-room",
        &source,
    );
    assert!(resolved.is_ok(), "{:?}", resolved.err());
    let chosen = rig.runtime.choose_notification_source(
        "local",
        enrolled,
        "another-room",
        std::slice::from_ref(&source),
    );
    assert!(chosen.is_ok(), "{:?}", chosen.err());
    assert!(rig
        .runtime
        .resolve_workflow_notification_source("local", "guest", "another-room", &source)
        .is_err());
    assert!(rig
        .runtime
        .choose_notification_source(
            "local",
            "guest",
            "another-room",
            std::slice::from_ref(&source)
        )
        .is_err());
}

// MP-08 / MP-09 / MP-10 / MP-11: the enrolled owner subscribes to its own App's event in a room
// made before enrollment; a collaborator still needs the App bound in the Room.
#[tokio::test]
async fn the_enrolled_owner_resolves_its_app_event_without_a_room_binding() {
    let rig = rig().await;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
        &rig.runtime.owned.durable_state_store,
        "local",
    );
    let enrolled = enroll(&rig);
    let resolved = rig
        .runtime
        .resolve_app_notification_source(
            "local",
            enrolled,
            &rig.session_id,
            catalog.installation_id(),
            "changed",
        )
        .await;
    assert!(resolved.is_ok(), "{:?}", resolved.err());
    let guest = rig
        .runtime
        .resolve_app_notification_source(
            "local",
            "guest",
            &rig.session_id,
            catalog.installation_id(),
            "changed",
        )
        .await;
    assert!(
        guest.as_ref().is_err_and(|error| error
            .to_string()
            .contains("App is not installed in this Room")),
        "{:?}",
        guest.err()
    );
}

// MP-08 / MP-10 (VS-S2-1): a source on another kernel delivers through the peer channel; the
// subscriber kernel admits it for the agent and queues it, and refuses once the trigger is gone.
#[tokio::test]
async fn a_remote_occurrence_is_admitted_for_the_agent_and_refused_without_its_trigger() {
    let rig = rig().await;
    let owner = "local";
    let source_id = "wf_source_remote";
    let trigger = rig
        .runtime
        .owned
        .session_store
        .write()
        .create_agent_notification_trigger(
            &rig.session_id,
            &rig.agent_id,
            owner,
            AgentTriggerNotificationSource::Workflow {
                source_id: source_id.into(),
            },
            "sub-remote".into(),
            "Studio pipeline".into(),
            None,
        )
        .unwrap();
    rig.runtime
        .owned
        .persist_agent_trigger_session(&rig.session_id, "agent_trigger_created")
        .unwrap();
    let home = rig.runtime.owned.config_projection.snapshot().daemon_id;
    let subscription = WorkflowNotificationSubscription {
        subscription_id: "sub-remote".into(),
        source_id: source_id.into(),
        owner_user_id: owner.into(),
        created_by_user_id: owner.into(),
        target_kernel_id: home,
        target_kind: WorkflowNotificationTargetKind::Agent,
        session_id: rig.session_id.clone(),
        workflow_id: String::new(),
        publication_id: trigger.id().into(),
        endpoint_id: rig.agent_id.clone(),
        queue_id: AGENT_QUEUE_ID.into(),
        source_available: true,
        source_kernel_id: "remote-kernel".into(),
        agent_id: Some(rig.agent_id.clone()),
        trigger_id: Some(trigger.id().into()),
    };
    rig.runtime
        .owned
        .durable_state_store
        .notify(NotificationOperation::RemoteAttach {
            subscription: subscription.clone(),
        })
        .unwrap();
    let envelope = |occurrence: &str, message: &str| crate::local::WorkflowNotificationEnvelope {
        source_id: source_id.into(),
        display_name: "Studio pipeline".into(),
        occurrence_id: occurrence.into(),
        output: WorkflowOutputPayload::new(message, vec![]),
        ancestry: vec!["[\"remote-kernel\",\"room\",\"wf\"]".into()],
        deadline_ms: crate::session::unix_epoch_ms() + 86_400_000,
    };
    let deliver = |occurrence: &str, message: &str| {
        rig.runtime.receive_workflow_notification_peer(
            "remote-kernel",
            owner,
            RelayPeerRequest::DeliverWorkflowNotification {
                protocol_version: VERSION,
                subscription_id: "sub-remote".into(),
                envelope: envelope(occurrence, message),
            },
        )
    };
    assert!(matches!(
        deliver("run-1", "shipped").unwrap(),
        RelayPeerResponse::WorkflowNotificationAccepted {
            ack: crate::local::WorkflowNotificationAck::Accepted
        }
    ));
    rig.runtime.route_agent_notifications().await;
    let queued = rig.queued_prompts();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(queued[0].contains("shipped") && queued[0].contains("Studio pipeline"));

    rig.runtime
        .owned
        .session_store
        .write()
        .delete_agent_trigger(&rig.session_id, trigger.id())
        .unwrap();
    assert!(
        deliver("run-2", "after the trigger is gone").is_err(),
        "the subscriber kernel refuses an occurrence for a trigger that is gone"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: App schema admission stays on the real App writer.
#[tokio::test]
async fn app_events_queue_each_occurrence_and_detach_when_the_agent_leaves() {
    use crate::durable_state::app_state::{AppStateOperation, AppStateOutcome};
    use chariox_app_runtime::app_outbox::{
        AutomationTargetKind, Invocation, Occurrence, ReceiptState,
    };
    let rig = rig().await;
    let store = &rig.runtime.owned.durable_state_store;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
    let mut request = rig.request("unused");
    request.source = AgentTriggerSourceRequest::Notification {
        source: AgentTriggerNotificationSource::App {
            installation_id: catalog.installation_id().into(),
            event_name: "changed".into(),
        },
    };
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = rig
        .runtime
        .create_agent_trigger(request, "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    let subscription = notification_subscription_id(&trigger).unwrap();
    let config = rig
        .runtime
        .owned
        .list_app_automations(
            "local",
            catalog.clone(),
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    assert_eq!(config[0].target.kind, AutomationTargetKind::Agent);
    let mut receipts = Vec::new();
    for index in 0..3 {
        let now = crate::session::unix_epoch_ms();
        let occurrence = Occurrence {
            automation_id: subscription.into(),
            occurrence_id: chariox_app_runtime::app_outbox::occurrence_id(
                &format!("issue-{index}"),
                now,
            )
            .unwrap(),
            event_version: 1,
            occurred_at_ms: now,
            schedule_revision: None,
            payload: serde_json::json!({"text":format!("issue-{index} </chariox-trigger-input>")}),
            invocation: Invocation {
                prompt: "Untrusted App hint".into(),
                artifacts: Vec::new(),
            },
        };
        let AppStateOutcome::Receipt(receipt) = store
            .execute_app_state(
                "local",
                catalog.clone(),
                AppStateOperation::Emit(occurrence),
                crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
            )
            .unwrap()
        else {
            panic!()
        };
        receipts.push(receipt);
    }
    // App receipts never enter workflow target resolution.
    assert!(store
        .pending_app_events("local", &catalog, crate::session::unix_epoch_ms(), 8)
        .unwrap()
        .is_empty());
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .unwrap();
    assert_eq!(rig.queued_prompts().len(), 3);
    for text in rig.queued_prompts() {
        assert!(text.contains("\\u003c/chariox-trigger-input\\u003e"));
        assert_eq!(text.matches("</chariox-trigger-input>").count(), 1);
    }
    for receipt in receipts {
        assert_eq!(
            store
                .app_event_candidate("local", catalog.clone(), &receipt.receipt_id)
                .unwrap()
                .receipt()
                .state,
            ReceiptState::Queued
        );
    }
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .unwrap();
    assert_eq!(rig.queued_prompts().len(), 3);
    rig.runtime
        .owned
        .session_store
        .write()
        .remove_agent_triggers_for_agent(&rig.session_id, &rig.agent_id)
        .unwrap();
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .unwrap();
    let config = rig
        .runtime
        .owned
        .list_app_automations(
            "local",
            catalog,
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    assert_eq!(
        config[0].status,
        chariox_app_runtime::app_outbox::AutomationStatus::Disabled
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: a workflow invoked from a notification keeps the agent in its lineage.
#[tokio::test]
async fn notification_lineage_follows_the_active_agent_prompt() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.complete(&workflow, "first", "released");
    rig.pass().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let state = &rig.runtime.owned.prompt_state_owner;
    state
        .complete_active_prompt_only(&room, &rig.agent_id)
        .unwrap();
    state
        .activate_next_queued_prompt(&room, &rig.agent_id, None)
        .unwrap();
    let invocation = rig
        .runtime
        .owned
        .inherit_agent_notification_ancestry(Some(&rig.agent_id), "entry", None)
        .unwrap()
        .unwrap();
    let ancestry: Vec<String> =
        serde_json::from_value(invocation.input["ancestry"].clone()).unwrap();
    assert_eq!(ancestry.last().unwrap(), &format!("agent:{}", rig.agent_id));
    let kernel = room.host_daemon_id();
    assert!(ancestry.contains(&store::workflow_identity(
        kernel,
        &rig.session_id,
        &workflow
    )));
    let prompt = state.active_prompt_for_agent(&room, &rig.agent_id).unwrap();
    assert_eq!(
        rig.runtime
            .owned
            .durable_state_store
            .agent_notification_ancestry(&rig.session_id, prompt.id())
            .unwrap()
            .unwrap()
            .len(),
        1
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: a source emits once per decision revision, through the shared router.
#[tokio::test]
async fn owner_action_source_queues_a_reduced_alert_once() {
    let rig = rig().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let source = rig
        .runtime
        .owned
        .ensure_owner_action_notification_source(&room)
        .unwrap();
    assert_eq!(
        source.kind,
        crate::local::NotificationSourceKind::OwnerAction
    );
    rig.runtime
        .create_agent_trigger(rig.request(&source.source_id), "local")
        .await
        .unwrap();
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let store = &rig.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: rig.session_id.clone(),
            agent: rig.agent_id.clone(),
            prompt: "blocked-source-drill".into(),
            run: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let Outcome::Task(blocked) = store
        .agent_lifecycle(Operation::Block {
            task: "blocked-source-drill".into(),
            prompt: "blocked-source-drill".into(),
            reason: "private reason excluded from notifications".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    for _ in 0..2 {
        rig.runtime
            .ensure_task_owner_interaction(blocked.clone())
            .await
            .unwrap();
    }
    rig.pass().await;
    assert_eq!(rig.queued_prompts().len(), 1);
    assert!(rig.queued_prompts()[0].contains("Agent needs your action"));
    assert!(!rig.queued_prompts()[0].contains("private reason excluded"));
}

// MP-08 / MP-09 / MP-10 / MP-11: after a Room ownership change only the new owner has the blocked-task source.
#[tokio::test]
async fn owner_action_source_follows_a_room_ownership_change() {
    let rig = rig().await;
    let store = &rig.runtime.owned.durable_state_store;
    let mut room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let former = rig
        .runtime
        .owned
        .ensure_owner_action_notification_source(&room)
        .unwrap();
    room.add_member(
        "heir",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    room.set_owner_user_id("heir");
    room.add_member(
        "local",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    rig.runtime
        .owned
        .session_store
        .write()
        .restore_session(room.clone());
    let current = rig
        .runtime
        .owned
        .ensure_owner_action_notification_source(&room)
        .unwrap();
    assert_eq!(current.owner_user_id, "heir");
    assert_ne!(current.source_id, former.source_id);
    let owner_action = |owner: &str| -> Vec<String> {
        store
            .notification_inventory(owner)
            .unwrap()
            .0
            .into_iter()
            .filter(|s| s.kind == crate::local::NotificationSourceKind::OwnerAction && s.enabled)
            .map(|s| s.source_id)
            .collect()
    };
    assert_eq!(owner_action("heir"), vec![current.source_id.clone()]);
    assert!(
        owner_action("local").is_empty(),
        "the former owner no longer gets it"
    );
    rig.runtime
        .create_agent_trigger(rig.request(&current.source_id), "local")
        .await
        .expect("the new owner's source resolves for a subscription");
    assert!(rig
        .runtime
        .create_agent_trigger(rig.request(&former.source_id), "local")
        .await
        .is_err());
}

// MP-08 / MP-09 / MP-10 / MP-11: replay after queue commit cannot duplicate an occurrence.
#[tokio::test]
async fn a_notification_queue_replay_reuses_the_admitted_prompt() {
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
    let payload = serde_json::json!({"source_id":source,"occurrence_id":"receipt-retry","output":"released","deadline_ms":crate::session::unix_epoch_ms()+crate::local::NOTIFICATION_RETENTION_MS});
    let first = rig
        .runtime
        .queue_agent_notification_input(
            &rig.session_id,
            trigger.id(),
            "wf_receipt_retry",
            payload.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    let second = rig
        .runtime
        .queue_agent_notification_input(&rig.session_id, trigger.id(), "wf_receipt_retry", payload)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(rig.queued_prompts().len(), 1);
}

// MP-08 / MP-09 / MP-10 / MP-11: a manual Steer carries lineage into the existing turn.
#[tokio::test]
async fn notification_lineage_follows_a_steered_prompt() {
    let (root, runtime, session_id, agent_id, attachment_id, active_prompt_id, _) =
        runtime_with_active_prompt().await;
    let rig = Rig {
        _root: root,
        runtime,
        session_id,
        agent_id,
        attachment_id: attachment_id.clone(),
    };
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.complete(&workflow, "steer-first", "released");
    rig.pass().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let (_, queued) = rig
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&room, &rig.agent_id);
    let prompt = queued[0].id().to_owned();
    assert!(rig
        .runtime
        .owned
        .durable_state_store
        .agent_notification_ancestry(&rig.session_id, &active_prompt_id)
        .unwrap()
        .is_none());
    rig.runtime
        .owned
        .steer_queued_prompt(&rig.session_id, &rig.agent_id, &attachment_id, &prompt)
        .unwrap();
    let invocation = rig
        .runtime
        .owned
        .inherit_agent_notification_ancestry(Some(&rig.agent_id), "entry", None)
        .unwrap()
        .unwrap();
    let ancestry: Vec<String> =
        serde_json::from_value(invocation.input["ancestry"].clone()).unwrap();
    assert!(ancestry.contains(&store::workflow_identity(
        room.host_daemon_id(),
        &rig.session_id,
        &workflow
    )));
    assert_eq!(ancestry.last().unwrap(), &format!("agent:{}", rig.agent_id));
}

impl Rig {
    /// An App event notifies the agent; its prompt is queued by the App agent router.
    async fn app_notification(&self) {
        use crate::durable_state::app_state::{AppStateOperation, AppStateOutcome};
        use chariox_app_runtime::app_outbox::{Invocation, Occurrence};
        let store = &self.runtime.owned.durable_state_store;
        let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
        let mut request = self.request("unused");
        request.source = AgentTriggerSourceRequest::Notification {
            source: AgentTriggerNotificationSource::App {
                installation_id: catalog.installation_id().into(),
                event_name: "changed".into(),
            },
        };
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = self
            .runtime
            .create_agent_trigger(request, "local")
            .await
            .unwrap()
        else {
            panic!()
        };
        let now = crate::session::unix_epoch_ms();
        let occurrence = Occurrence {
            automation_id: notification_subscription_id(&trigger).unwrap().into(),
            occurrence_id: chariox_app_runtime::app_outbox::occurrence_id("issue", now).unwrap(),
            event_version: 1,
            occurred_at_ms: now,
            schedule_revision: None,
            payload: serde_json::json!({"text":"issue"}),
            invocation: Invocation {
                prompt: "App hint".into(),
                artifacts: Vec::new(),
            },
        };
        let AppStateOutcome::Receipt(_) = store
            .execute_app_state(
                "local",
                catalog.clone(),
                AppStateOperation::Emit(occurrence),
                crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
            )
            .unwrap()
        else {
            panic!()
        };
        self.runtime
            .route_app_agent_installation("local", catalog)
            .await
            .unwrap();
    }

    fn lineage_of_the_active_agent(&self) -> Vec<String> {
        let invocation = self
            .runtime
            .owned
            .inherit_agent_notification_ancestry(Some(&self.agent_id), "entry", None)
            .unwrap()
            .expect("the active prompt came from a notification");
        serde_json::from_value(invocation.input["ancestry"].clone()).unwrap()
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: an App-notified agent is part of the lineage of the workflows it invokes.
#[tokio::test]
async fn an_app_notified_agent_cannot_be_notified_back_through_a_workflow_it_invoked() {
    let rig = rig().await;
    rig.app_notification().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let state = &rig.runtime.owned.prompt_state_owner;
    state
        .complete_active_prompt_only(&room, &rig.agent_id)
        .unwrap();
    state
        .activate_next_queued_prompt(&room, &rig.agent_id, None)
        .unwrap();
    assert_eq!(
        rig.lineage_of_the_active_agent(),
        vec![format!("agent:{}", rig.agent_id)]
    );
    let workflow = rig.workflow_with_exit("reply");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    let invocation = rig
        .runtime
        .owned
        .inherit_agent_notification_ancestry(Some(&rig.agent_id), "entry", None)
        .unwrap();
    rig.complete_with_invocation(&workflow, "cycle", "done", invocation.clone());
    rig.pass().await;
    assert!(
        rig.queued_prompts().is_empty(),
        "A -> W -> A must be dropped"
    );
    let mut unrelated = invocation.unwrap();
    unrelated.input["ancestry"] = serde_json::json!(["agent:someone-else"]);
    rig.complete_with_invocation(&workflow, "unrelated", "done", Some(unrelated));
    rig.pass().await;
    assert_eq!(
        rig.queued_prompts().len(),
        1,
        "another lineage is still notified"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: an App notification steered into a running turn joins that turn's lineage.
#[tokio::test]
async fn an_app_notification_steered_into_a_turn_keeps_the_agent_in_its_lineage() {
    let rig = rig().await;
    rig.app_notification().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let (_, queued) = rig
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&room, &rig.agent_id);
    let prompt = queued[0].id().to_owned();
    rig.runtime
        .owned
        .steer_queued_prompt(&rig.session_id, &rig.agent_id, &rig.attachment_id, &prompt)
        .unwrap();
    assert_eq!(
        rig.lineage_of_the_active_agent(),
        vec![format!("agent:{}", rig.agent_id)]
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: the picker cannot expose another Room's Apps to a guest.
#[tokio::test]
async fn room_app_source_listing_is_member_scoped() {
    let rig = rig().await;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
        &rig.runtime.owned.durable_state_store,
        "local",
    );
    let mut room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    room.add_member(
        "guest",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    rig.runtime
        .owned
        .session_store
        .write()
        .restore_session(room);
    let request =
        LocalDaemonRequest::ListAppInstallations(crate::local::ListAppInstallationsRequest {
            session_id: Some(rig.session_id.clone()),
            after: None,
            limit: None,
        });
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "room-app-sources",
        None,
        None,
        &request,
    );
    command.caller.user_id = Some("guest".into());
    let Some(LocalDaemonResponse::AppInstallationsListed { installations, .. }) = rig
        .runtime
        .execute_app_control_request(&command, &request)
        .await
    else {
        panic!("expected a scoped list")
    };
    assert!(
        installations.is_empty(),
        "an unbound owner App is not the Room's"
    );
    rig.runtime
        .owned
        .agent_store
        .grant_extension(
            &rig.agent_id,
            crate::extension::ExtensionGrant::app(catalog.installation_id()),
        )
        .unwrap();
    let Some(LocalDaemonResponse::AppInstallationsListed { installations, .. }) = rig
        .runtime
        .execute_app_control_request(&command, &request)
        .await
    else {
        panic!("expected a scoped list")
    };
    assert_eq!(installations.len(), 1);
    assert_eq!(installations[0].installation_id, catalog.installation_id());
    assert_eq!(installations[0].events[0].name, "changed");
    command.caller.user_id = Some("outsider".into());
    assert!(matches!(
        rig.runtime
            .execute_app_control_request(&command, &request)
            .await,
        Some(LocalDaemonResponse::AppRequestFailed {
            code: crate::local::AppRequestErrorCode::Unauthorized
        })
    ));
}

// MP-08 / MP-09 / MP-10 / MP-11: a Room context grants no control of another member's App binding.
#[tokio::test]
async fn guest_app_automation_listing_and_removal_keep_target_ownership() {
    let rig = rig().await;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
        &rig.runtime.owned.durable_state_store,
        "local",
    );
    rig.runtime
        .owned
        .agent_store
        .grant_extension(
            &rig.agent_id,
            crate::extension::ExtensionGrant::app(catalog.installation_id()),
        )
        .unwrap();
    let mut room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    room.add_member(
        "guest",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    rig.runtime
        .owned
        .session_store
        .write()
        .restore_session(room);
    let mut create = rig.request("unused");
    create.source = AgentTriggerSourceRequest::Notification {
        source: AgentTriggerNotificationSource::App {
            installation_id: catalog.installation_id().into(),
            event_name: "changed".into(),
        },
    };
    rig.runtime
        .create_agent_trigger(create, "local")
        .await
        .unwrap();
    let list = LocalDaemonRequest::ListAppAutomations(crate::local::ListAppAutomationsRequest {
        installation_id: catalog.installation_id().into(),
        session_id: Some(rig.session_id.clone()),
    });
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "scoped-app-bindings",
        None,
        None,
        &list,
    );
    command.caller.user_id = Some("guest".into());
    let Some(LocalDaemonResponse::AppAutomations { automations, .. }) = rig
        .runtime
        .execute_app_control_request(&command, &list)
        .await
    else {
        panic!("expected a scoped list")
    };
    assert!(automations.is_empty());
    command.caller.user_id = Some("local".into());
    let Some(LocalDaemonResponse::AppAutomations { automations, .. }) = rig
        .runtime
        .execute_app_control_request(&command, &list)
        .await
    else {
        panic!("expected owner's binding")
    };
    assert_eq!(automations.len(), 1);
    let row = &automations[0];
    let disable =
        LocalDaemonRequest::DisableAppAutomation(crate::local::DisableAppAutomationRequest {
            session_id: Some(rig.session_id.clone()),
            installation_id: catalog.installation_id().into(),
            automation_id: row.automation_id.clone(),
            expected_revision: row.revision,
        });
    command.caller.user_id = Some("guest".into());
    assert!(matches!(
        rig.runtime
            .execute_app_control_request(&command, &disable)
            .await,
        Some(LocalDaemonResponse::AppRequestFailed {
            code: crate::local::AppRequestErrorCode::Unauthorized
        })
    ));
    command.caller.user_id = Some("local".into());
    let Some(LocalDaemonResponse::AppAutomation { automation, .. }) = rig
        .runtime
        .execute_app_control_request(&command, &disable)
        .await
    else {
        panic!("owner can disable their binding")
    };
    assert_eq!(
        automation.status,
        crate::local::AppAutomationStatus::Disabled
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: App receipts share ordinary prompt lifecycle and retention.
impl Rig {
    async fn app_trigger(
        &self,
        catalog: &chariox_app_runtime::app_outbox::EventCatalog,
    ) -> crate::session::AgentTrigger {
        let mut request = self.request("unused");
        request.source = AgentTriggerSourceRequest::Notification {
            source: AgentTriggerNotificationSource::App {
                installation_id: catalog.installation_id().into(),
                event_name: "changed".into(),
            },
        };
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = self
            .runtime
            .create_agent_trigger(request, "local")
            .await
            .unwrap()
        else {
            panic!("App trigger")
        };
        trigger
    }
    fn emit_app(
        &self,
        catalog: std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>,
        trigger: &crate::session::AgentTrigger,
        label: &str,
    ) -> chariox_app_runtime::app_outbox::Receipt {
        use crate::durable_state::app_state::{AppStateOperation, AppStateOutcome};
        use chariox_app_runtime::app_outbox::{occurrence_id, Invocation, Occurrence};
        let now = crate::session::unix_epoch_ms();
        let occurrence = Occurrence {
            automation_id: notification_subscription_id(trigger).unwrap().into(),
            occurrence_id: occurrence_id(label, now).unwrap(),
            event_version: 1,
            occurred_at_ms: now,
            schedule_revision: None,
            payload: serde_json::json!({"text":label}),
            invocation: Invocation {
                prompt: "App hint".into(),
                artifacts: vec![],
            },
        };
        let AppStateOutcome::Receipt(receipt) = self
            .runtime
            .owned
            .durable_state_store
            .execute_app_state(
                "local",
                catalog,
                AppStateOperation::Emit(occurrence),
                crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
            )
            .unwrap()
        else {
            panic!("receipt")
        };
        receipt
    }
}

#[tokio::test]
async fn app_agent_receipts_settle_on_completion_and_cancellation_and_are_pruned() {
    use chariox_app_runtime::app_outbox::ReceiptState;
    let rig = rig().await;
    let store = &rig.runtime.owned.durable_state_store;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
    let trigger = rig.app_trigger(&catalog).await;
    let completed = rig.emit_app(catalog.clone(), &trigger, "complete");
    let cancelled = rig.emit_app(catalog.clone(), &trigger, "cancel");
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .unwrap();
    let receipt = |id: &str| {
        store
            .app_event_candidate("local", catalog.clone(), id)
            .unwrap()
            .receipt()
            .clone()
    };
    let queued = receipt(&cancelled.receipt_id).queued_prompt_id.unwrap();
    rig.runtime
        .owned
        .cancel_queued_prompt(&rig.session_id, &rig.agent_id, &rig.attachment_id, &queued)
        .unwrap();
    assert_eq!(
        receipt(&cancelled.receipt_id).state,
        ReceiptState::Failed,
        "cancelled queued notification becomes terminal"
    );
    // Completing the initial user prompt promotes the first App notification.
    rig.runtime
        .complete_agent_prompt_with_external_authority(&rig.session_id, &rig.agent_id, None, None)
        .await
        .unwrap();
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    if room.active_prompt_for_agent(&rig.agent_id).is_none() {
        rig.runtime
            .owned
            .activate_next_queued_prompt_for_agent(&rig.session_id, &rig.agent_id, None)
            .unwrap();
    }
    rig.runtime
        .complete_agent_prompt_with_external_authority(&rig.session_id, &rig.agent_id, None, None)
        .await
        .unwrap();
    assert_eq!(
        receipt(&completed.receipt_id).state,
        ReceiptState::Delivered,
        "completed notification becomes terminal"
    );
    let mut connection = rusqlite::Connection::open(store.path()).unwrap();
    let mut tx = connection.transaction().unwrap();
    let result = chariox_app_runtime::app_outbox::AppOutbox::maintain_in(
        &mut tx,
        "local",
        catalog.installation_id(),
        completed.expires_at_ms + chariox_app_runtime::app_outbox::MAX_OCCURRENCE_AGE_MS + 1,
    )
    .unwrap();
    assert_eq!(
        result.pruned, 2,
        "terminal App-to-agent receipts release installation capacity"
    );
    tx.commit().unwrap();
}

#[tokio::test]
async fn disabled_app_subscriber_does_not_starve_a_live_subscriber() {
    for removed in [false, true] {
        let rig = rig().await;
        let store = &rig.runtime.owned.durable_state_store;
        let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
        let disabled = rig.app_trigger(&catalog).await;
        let live = rig.app_trigger(&catalog).await;
        let mut deferred = Vec::new();
        for n in 0..8 {
            deferred.push(rig.emit_app(catalog.clone(), &disabled, &format!("disabled-{n}")));
        }
        let live_receipt = rig.emit_app(catalog.clone(), &live, "live-subscriber");
        if removed {
            rig.runtime
                .delete_agent_trigger(
                    DeleteAgentTriggerRequest {
                        session_id: rig.session_id.clone(),
                        trigger_id: disabled.id().into(),
                    },
                    "local",
                )
                .await
                .unwrap();
        } else {
            rig.runtime
                .update_agent_trigger(
                    UpdateAgentTriggerRequest {
                        session_id: rig.session_id.clone(),
                        trigger_id: disabled.id().into(),
                        busy_policy: None,
                        prompt: None,
                        enabled: Some(false),
                    },
                    "local",
                )
                .await
                .unwrap();
        }
        rig.runtime
            .route_app_agent_installation("local", catalog.clone())
            .await
            .unwrap();
        // Fairness must survive a slow pump too, after the deferred page becomes due again.
        let pending = store
            .pending_app_events_for_target(
                "local",
                &catalog,
                crate::session::unix_epoch_ms().saturating_add(5_000),
                8,
                chariox_app_runtime::app_outbox::AutomationTargetKind::Agent,
            )
            .unwrap();
        assert_eq!(
            pending[0].receipt_id, live_receipt.receipt_id,
            "deferred rows rotate behind eligible live work even on a slow pass"
        );
        rig.runtime
            .route_app_agent_installation("local", catalog.clone())
            .await
            .unwrap();
        assert_eq!(
            rig.queued_prompts().len(),
            1,
            "the next bounded pass must reach the enabled subscriber"
        );
        assert!(rig.queued_prompts()[0].contains("live-subscriber"));
        for original in deferred {
            let current = store
                .app_event_candidate("local", catalog.clone(), &original.receipt_id)
                .unwrap();
            assert_eq!(current.receipt().expires_at_ms, original.expires_at_ms);
            assert_eq!(
                current.receipt().attempts,
                0,
                "deferral is not a failed delivery attempt"
            );
        }
    }
}

#[tokio::test]
async fn app_agent_receipt_settles_when_prompt_cancellation_precedes_association() {
    use crate::durable_state::app_automations::AppAutomationMutation;
    use chariox_app_runtime::app_outbox::ReceiptState;
    let rig = rig().await;
    let store = &rig.runtime.owned.durable_state_store;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
    let trigger = rig.app_trigger(&catalog).await;
    let receipt = rig.emit_app(catalog.clone(), &trigger, "fast-cancellation");
    let prompt_id = rig
        .runtime
        .queue_agent_notification_input(
            &rig.session_id,
            trigger.id(),
            &receipt.receipt_id,
            serde_json::json!({"installation_id":catalog.installation_id(),"event_name":"changed","occurrence_id":receipt.occurrence_id,"occurred_at_ms":receipt.occurred_at_ms,"deadline_ms":receipt.expires_at_ms,"payload":{"text":"fast-cancellation"}}),
        )
        .await
        .unwrap()
        .unwrap();
    rig.runtime
        .owned
        .cancel_queued_prompt(
            &rig.session_id,
            &rig.agent_id,
            &rig.attachment_id,
            &prompt_id,
        )
        .unwrap();
    store
        .mutate_app_automation(
            "local",
            catalog.clone(),
            AppAutomationMutation::AgentQueued {
                receipt: receipt.clone(),
                prompt_id,
            },
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    assert_eq!(
        store
            .app_event_candidate("local", catalog, &receipt.receipt_id)
            .unwrap()
            .receipt()
            .state,
        ReceiptState::Failed,
        "late receipt association must observe committed prompt cancellation"
    );
}

#[tokio::test]
async fn r6_full_app_subscriber_does_not_starve_an_available_subscriber() {
    let rig = rig().await;
    let store = &rig.runtime.owned.durable_state_store;
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
    let full = rig.app_trigger(&catalog).await;
    let mut room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let mut other = serde_json::to_value(
        rig.runtime
            .owned
            .agent_store
            .get_agent(&rig.agent_id)
            .unwrap(),
    )
    .unwrap();
    other["id"] = serde_json::json!("available-agent");
    other["agent_ref"] = serde_json::json!("available-agent-ref");
    let other: crate::agent::AgentInstance = serde_json::from_value(other).unwrap();
    rig.runtime.owned.agent_store.restore_agent(other.clone());
    let mut agents = room.agents().to_vec();
    agents.push(other.clone());
    room.set_agents(agents);
    room.mirror_agent_prompt_state(
        other.id(),
        Some(crate::session::PromptQueueItem::new(
            "available-active",
            &rig.attachment_id,
            other.id(),
            "holding",
            crate::session::PromptStatus::Running,
        )),
        Default::default(),
    );
    rig.runtime
        .owned
        .session_store
        .write()
        .restore_session(room.clone());
    rig.runtime
        .owned
        .prompt_state_owner
        .restore_session_state(&room);
    let mut request = rig.request("unused");
    request.agent_id = other.id().into();
    request.source = AgentTriggerSourceRequest::Notification {
        source: AgentTriggerNotificationSource::App {
            installation_id: catalog.installation_id().into(),
            event_name: "changed".into(),
        },
    };
    let LocalDaemonResponse::AgentTriggerCreated {
        trigger: available, ..
    } = rig
        .runtime
        .create_agent_trigger(request, "local")
        .await
        .unwrap()
    else {
        panic!()
    };
    for n in 0..crate::runtime::prompt_state::PROMPT_QUEUE_LIMIT {
        rig.runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(
                &room,
                crate::session::PromptQueueItem::new(
                    format!("full-{n}"),
                    &rig.attachment_id,
                    &rig.agent_id,
                    "ordinary backlog",
                    crate::session::PromptStatus::Queued,
                ),
                true,
            )
            .unwrap();
    }
    let blocked = rig.emit_app(catalog.clone(), &full, "full-subscriber");
    let live = rig.emit_app(catalog.clone(), &available, "available-subscriber");
    rig.runtime
        .route_app_agent_installation("local", catalog.clone())
        .await
        .expect("one full queue must not abort an installation pass");
    let current = store
        .app_event_candidate("local", catalog.clone(), &blocked.receipt_id)
        .unwrap();
    assert_eq!(current.receipt().expires_at_ms, blocked.expires_at_ms);
    assert_eq!(current.receipt().attempts, 0);
    assert!(
        store
            .app_event_candidate("local", catalog, &live.receipt_id)
            .unwrap()
            .receipt()
            .queued_prompt_id
            .is_some(),
        "the independent subscriber is reached in the same bounded pass"
    );
}

#[tokio::test]
async fn r6_notification_deadline_survives_queue_recovery_and_blocks_promotion_and_steer() {
    for app in [false, true] {
        for steer in [false, true] {
            let config = super::super::agent_trigger_runtime_state::tests::durable_state_config(
                "r6-notification-expiry",
            );
            let (root, runtime, session_id, agent_id, attachment_id, _, _) = super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(config.clone(), &[]).await;
            let mut rig = Rig {
                _root: root,
                runtime,
                session_id,
                agent_id,
                attachment_id,
            };
            let expected_deadline = if app {
                let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
                    &rig.runtime.owned.durable_state_store,
                    "local",
                );
                let trigger = rig.app_trigger(&catalog).await;
                let receipt = rig.emit_app(catalog.clone(), &trigger, "expiry");
                rig.runtime
                    .route_app_agent_installation("local", catalog)
                    .await
                    .unwrap();
                Some(receipt.expires_at_ms)
            } else {
                let workflow = rig.workflow_with_exit("expiry-pipeline");
                let source = rig.register(&workflow);
                rig.runtime
                    .create_agent_trigger(rig.request(&source), "local")
                    .await
                    .unwrap();
                rig.complete(&workflow, "expiry-run", "expiry");
                rig.pass().await;
                None
            };
            let mut room = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
            let (_, queued) = rig
                .runtime
                .owned
                .prompt_state_owner
                .state_parts(&room, &rig.agent_id);
            assert_eq!(queued.len(), 1);
            let id = queued[0].id().to_owned();
            let mut bytes = serde_json::to_value(
                crate::durable_prompt_state::DurablePromptStateEventPayload::capture(
                    &room,
                    &rig.agent_id,
                ),
            )
            .unwrap();
            let private = bytes["private_states"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|value| value["prompt_id"] == id)
                .unwrap();
            let deadline = private["notification_deadline_ms"].as_u64().expect(
                "the kernel owns a durable notification deadline, beyond quoted prompt text",
            );
            assert!(deadline > crate::session::unix_epoch_ms());
            if let Some(expected) = expected_deadline {
                assert_eq!(deadline, expected);
            }
            private["notification_deadline_ms"] =
                serde_json::json!(crate::session::unix_epoch_ms().saturating_sub(1));
            let encoded = serde_json::to_vec(&bytes).unwrap();
            let mut recovered: crate::durable_prompt_state::DurablePromptStateEventPayload =
                serde_json::from_slice(&encoded).unwrap();
            recovered.restore_private_states();
            room.mirror_agent_prompt_state(
                &rig.agent_id,
                if steer { recovered.active_prompt } else { None },
                recovered.queued_prompts,
            );
            rig.runtime
                .owned
                .session_store
                .write()
                .restore_session(room.clone());
            rig.runtime
                .owned
                .prompt_state_owner
                .restore_session_state(&room);
            rig.runtime
                .owned
                .persist_prompt_session_state(&room, &rig.agent_id)
                .unwrap();
            if steer {
                rig.runtime
                    .with_app_side_effect(|app| app.save_durable_state_snapshot())
                    .await
                    .unwrap();
            }
            let mut restart_config = config.clone();
            let restart_path =
                super::super::agent_trigger_runtime_state::tests::durable_state_config(
                    "r6-notification-expiry-restart",
                )
                .user_config
                .state
                .path
                .unwrap();
            rusqlite::Connection::open(rig.runtime.owned.durable_state_store.path())
                .unwrap()
                .execute("VACUUM INTO ?1", rusqlite::params![restart_path])
                .unwrap();
            restart_config.user_config.state.path = Some(restart_path);
            let app = crate::app::DaemonApp::bootstrap(restart_config).unwrap();
            rig.runtime = super::super::local_prompt_dispatch_runtime::tests::owned_runtime_state(
                &std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            )
            .await;
            assert_eq!(rig.queued_prompts().len(), 1, "the real kernel bootstrap restores the expired queued item and its private metadata");
            if steer {
                assert!(
                    rig.runtime
                        .owned
                        .steer_queued_prompt(
                            &rig.session_id,
                            &rig.agent_id,
                            &rig.attachment_id,
                            &id
                        )
                        .is_err(),
                    "expired occurrence must not steer after recovery"
                );
            } else {
                assert!(
                    rig.runtime
                        .owned
                        .advance_queued_prompt_dispatch_for_head(
                            &rig.session_id,
                            &rig.agent_id,
                            "expired-provider",
                            None
                        )
                        .unwrap()
                        .is_none(),
                    "expired occurrence must not promote after recovery"
                );
            }
            assert!(
                rig.queued_prompts().is_empty(),
                "expiry removes the queued row durably"
            );
            let room = rig
                .runtime
                .owned
                .session_store
                .get_session(&rig.session_id)
                .unwrap();
            assert!(room
                .queued_prompts_for_agent(&rig.agent_id)
                .is_none_or(|queued| queued.is_empty()));
        }
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: a worker may act before its Steer acknowledgement arrives.
impl Rig {
    fn remote_steer_intent(
        &self,
        prompt: &str,
    ) -> (
        super::prompt_queue_owned_state::RemoteQueuedPromptSteerPreparation,
        u64,
    ) {
        let binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "notification-worker".into(),
            worker_machine_id: "notification-machine".into(),
            execution_lease_id: "notification-lease".into(),
            leased_agent_id: "notification-agent".into(),
            active_worker_provider_run_id: Some("notification-run".into()),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
            worker_worktree_id: None,
        };
        self.runtime
            .owned
            .agent_store
            .bind_remote_execution(&self.agent_id, binding.clone())
            .unwrap();
        let prepared = self
            .runtime
            .owned
            .prepare_remote_queued_prompt_steer(
                &self.session_id,
                &self.agent_id,
                &self.attachment_id,
                prompt,
                None,
            )
            .unwrap()
            .unwrap();
        let reservation = self
            .runtime
            .owned
            .reserve_remote_queued_prompt_steer(
                &self.session_id,
                &self.agent_id,
                &self.attachment_id,
                prompt,
                &prepared,
            )
            .unwrap();
        self.runtime
            .owned
            .mark_remote_queued_prompt_steer_uncertain(
                &self.session_id,
                &self.agent_id,
                &prepared.prompt,
                reservation,
                prepared.target_active_prompt_id.clone(),
                binding.active_worker_provider_run_id,
                binding.worker_kernel_id,
                binding.worker_machine_id,
                binding.execution_lease_id,
                binding.leased_agent_id,
            )
            .unwrap();
        (prepared, reservation)
    }
}

#[tokio::test]
async fn r7_remote_notification_lineage_precedes_a_lost_steer_acknowledgement() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.complete(&workflow, "remote-first", "released");
    rig.pass().await;
    let room = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session_id)
        .unwrap();
    let queued = rig
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&room, &rig.agent_id)
        .1[0]
        .id()
        .to_owned();
    let (prepared, reservation) = rig.remote_steer_intent(&queued);
    // The worker accepted the send, but its acknowledgement is lost: do not call finish.
    prepared.prompt.release_remote_steer(reservation);
    assert!(prepared.prompt.remote_steer_outcome_uncertainty().is_some());
    let invocation = rig
        .runtime
        .owned
        .inherit_agent_notification_ancestry(Some(&rig.agent_id), "entry", None)
        .unwrap()
        .expect("the worker can invoke a workflow before its Steer acknowledgement");
    let ancestry: Vec<String> =
        serde_json::from_value(invocation.input["ancestry"].clone()).unwrap();
    assert!(ancestry.contains(&format!("agent:{}", rig.agent_id)));
    rig.complete_with_invocation(
        &workflow,
        "remote-loop",
        "must be dropped",
        Some(invocation),
    );
    rig.pass().await;
    assert_eq!(
        rig.queued_prompts().len(),
        1,
        "the causal loop cannot notify the same agent again"
    );
    assert_eq!(
        rig.runtime
            .owned
            .durable_state_store
            .agent_notification_ancestry(&rig.session_id, &prepared.target_active_prompt_id)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: a proven rejection takes back the lineage given to the active turn.
#[tokio::test]
async fn a_proven_remote_steer_rejection_removes_its_provisional_lineage() {
    for exact_receipt in [false, true] {
        let rig = rig().await;
        let workflow = rig.workflow_with_exit("pipeline");
        let source = rig.register(&workflow);
        rig.runtime
            .create_agent_trigger(rig.request(&source), "local")
            .await
            .unwrap();
        rig.complete(&workflow, "first", "released");
        rig.pass().await;
        let room = rig
            .runtime
            .owned
            .session_store
            .get_session(&rig.session_id)
            .unwrap();
        let queued = rig
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&room, &rig.agent_id)
            .1[0]
            .id()
            .to_owned();
        let (prepared, reservation) = rig.remote_steer_intent(&queued);
        let store = &rig.runtime.owned.durable_state_store;
        assert!(
            store
                .agent_notification_ancestry(&rig.session_id, &prepared.target_active_prompt_id)
                .unwrap()
                .is_some(),
            "the send intent binds the lineage before the worker answers"
        );
        if exact_receipt {
            rig.runtime
                .owned
                .reconcile_remote_queued_prompt_steer_receipt(
                    &rig.session_id,
                    &rig.agent_id,
                    &queued,
                    &crate::transport::relay_peer::LeasedPromptReceipt {
                        home_prompt_id: queued.clone(),
                        worker_provider_run_id: "notification-run".into(),
                        phase:
                            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected,
                        target_home_prompt_id: Some(prepared.target_active_prompt_id.clone()),
                        execution_lease_id: Some("notification-lease".into()),
                    },
                )
                .unwrap();
        } else {
            rig.runtime
                .owned
                .clear_remote_queued_prompt_steer_uncertainty(
                    &rig.session_id,
                    &rig.agent_id,
                    &prepared.prompt,
                    reservation,
                )
                .unwrap();
        }
        assert!(
            store
                .agent_notification_ancestry(&rig.session_id, &prepared.target_active_prompt_id)
                .unwrap()
                .is_none(),
            "the active turn never received the input (exact receipt: {exact_receipt})"
        );
        assert!(rig
            .runtime
            .owned
            .inherit_agent_notification_ancestry(Some(&rig.agent_id), "entry", None)
            .unwrap()
            .is_none());
        rig.complete(&workflow, "after-rejection", "released again");
        rig.pass().await;
        assert_eq!(
            rig.queued_prompts().len(),
            2,
            "a workflow the original turn invoked still notifies (exact receipt: {exact_receipt})"
        );
    }
}

#[tokio::test]
async fn r7_app_agent_receipts_recognize_local_and_remote_general_steer() {
    use crate::durable_state::app_automations::AppAutomationMutation;
    use chariox_app_runtime::app_outbox::ReceiptState;
    for mode in ["local", "remote", "remote_recovered"] {
        for late_association in [false, true] {
            let rig = rig().await;
            let store = &rig.runtime.owned.durable_state_store;
            let catalog =
                crate::durable_state::app_state::fixture_room_event_catalog(store, "local");
            let trigger = rig.app_trigger(&catalog).await;
            let emitted = rig.emit_app(catalog.clone(), &trigger, "general-steer");
            let queued = if late_association {
                rig.runtime.queue_agent_notification_input(&rig.session_id, trigger.id(), &emitted.receipt_id, serde_json::json!({"installation_id":catalog.installation_id(),"event_name":"changed","occurrence_id":emitted.occurrence_id,"occurred_at_ms":emitted.occurred_at_ms,"deadline_ms":emitted.expires_at_ms,"payload":{"text":"general-steer"}})).await.unwrap().unwrap()
            } else {
                rig.runtime
                    .route_app_agent_installation("local", catalog.clone())
                    .await
                    .unwrap();
                store
                    .app_event_candidate("local", catalog.clone(), &emitted.receipt_id)
                    .unwrap()
                    .receipt()
                    .queued_prompt_id
                    .clone()
                    .unwrap()
            };
            if mode == "local" {
                let prepared = rig
                    .runtime
                    .owned
                    .prepare_local_queued_prompt_steer(
                        &rig.session_id,
                        &rig.agent_id,
                        &rig.attachment_id,
                        &queued,
                    )
                    .unwrap();
                let sequence = rig
                    .runtime
                    .owned
                    .record_local_queued_prompt_steer(&prepared)
                    .unwrap();
                rig.runtime
                    .owned
                    .finish_local_queued_prompt_steer(&prepared, sequence)
                    .unwrap();
            } else {
                let (prepared, reservation) = rig.remote_steer_intent(&queued);
                if mode == "remote" {
                    rig.runtime
                        .owned
                        .finish_remote_queued_prompt_steer(
                            &rig.session_id,
                            &rig.agent_id,
                            &rig.attachment_id,
                            &prepared,
                            &prepared.remote_execution,
                            reservation,
                            &prepared.provider_run_id,
                        )
                        .unwrap();
                } else {
                    prepared.prompt.release_remote_steer(reservation);
                    let receipt = crate::transport::relay_peer::LeasedPromptReceipt {
                        home_prompt_id: queued.clone(),
                        worker_provider_run_id: prepared.provider_run_id.clone(),
                        phase:
                            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerAccepted,
                        target_home_prompt_id: Some(prepared.target_active_prompt_id.clone()),
                        execution_lease_id: Some(
                            prepared.remote_execution.execution_lease_id.clone(),
                        ),
                    };
                    assert!(matches!(rig.runtime.owned.reconcile_remote_queued_prompt_steer_receipt(&rig.session_id, &rig.agent_id, &queued, &receipt).unwrap(), super::prompt_queue_owned_state::RemoteQueuedPromptSteerReceiptSettlement::Accepted));
                }
            }
            if late_association {
                store
                    .mutate_app_automation(
                        "local",
                        catalog.clone(),
                        AppAutomationMutation::AgentQueued {
                            receipt: emitted.clone(),
                            prompt_id: queued,
                        },
                        crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(
                            || false,
                        ),
                    )
                    .unwrap();
            }
            assert_eq!(store.app_event_candidate("local", catalog.clone(), &emitted.receipt_id).unwrap().receipt().state, ReceiptState::Delivered, "general Steer delivered the App notification ({mode}, late_association={late_association})");
            assert_eq!(
                rig.lineage_of_the_active_agent(),
                vec![format!("agent:{}", rig.agent_id)],
                "the steered turn keeps the App notification's lineage ({mode}, late_association={late_association})"
            );
            let workflow = rig.workflow_with_exit("reply");
            let source = rig.register(&workflow);
            rig.runtime
                .create_agent_trigger(rig.request(&source), "local")
                .await
                .unwrap();
            rig.assert_the_cycle_is_dropped(&workflow).await;
        }
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: settling the receipt after a Steer keeps the steered turn's lineage.
#[tokio::test]
async fn a_steer_before_the_receipt_settles_keeps_the_steered_turn_lineage() {
    for remote in [false, true] {
        let rig = rig().await;
        let workflow = rig.workflow_with_exit("pipeline");
        let source = rig.register(&workflow);
        rig.runtime
            .create_agent_trigger(rig.request(&source), "local")
            .await
            .unwrap();
        rig.complete(&workflow, "steer-early", "released");
        let (subscription, envelope, queued) = rig.admit_workflow_notification_unsettled().await;
        if remote {
            let (prepared, reservation) = rig.remote_steer_intent(&queued);
            rig.runtime
                .owned
                .finish_remote_queued_prompt_steer(
                    &rig.session_id,
                    &rig.agent_id,
                    &rig.attachment_id,
                    &prepared,
                    &prepared.remote_execution,
                    reservation,
                    &prepared.provider_run_id,
                )
                .unwrap();
        } else {
            rig.runtime
                .owned
                .steer_queued_prompt(&rig.session_id, &rig.agent_id, &rig.attachment_id, &queued)
                .unwrap();
        }
        rig.runtime
            .settle_agent_notification(&subscription, &envelope, &queued)
            .unwrap();
        let lineage = rig.lineage_of_the_active_agent();
        assert_eq!(lineage.last().unwrap(), &format!("agent:{}", rig.agent_id));
        let room = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
        assert!(
            lineage.contains(&store::workflow_identity(
                room.host_daemon_id(),
                &rig.session_id,
                &workflow
            )),
            "remote={remote}"
        );
        rig.assert_the_cycle_is_dropped(&workflow).await;
    }
}

impl Rig {
    /// The notification pump up to the old association point: the prompt for the
    /// accepted occurrence is admitted, its receipt is not settled yet.
    async fn admit_workflow_notification_unsettled(
        &self,
    ) -> (
        WorkflowNotificationSubscription,
        crate::local::WorkflowNotificationEnvelope,
        String,
    ) {
        self.runtime.owned.route_workflow_notifications();
        let store = &self.runtime.owned.durable_state_store;
        let (subscription, envelope) = store
            .notification_candidates(true, crate::session::unix_epoch_ms(), 8)
            .unwrap()
            .into_iter()
            .find(|(subscription, _)| {
                subscription.target_kind == WorkflowNotificationTargetKind::Agent
            })
            .expect("an accepted occurrence for the agent");
        let receipt = store::receipt_id(&subscription, &envelope).unwrap();
        let prompt = self
            .runtime
            .queue_agent_notification_input(
                &subscription.session_id,
                subscription.trigger_id.as_deref().unwrap(),
                &receipt,
                serde_json::to_value(&envelope).unwrap(),
            )
            .await
            .unwrap()
            .expect("the notification prompt is admitted");
        (subscription, envelope, prompt)
    }

    /// The App router up to its old association point.
    async fn admit_app_notification_unsettled(
        &self,
    ) -> (
        std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>,
        chariox_app_runtime::app_outbox::Receipt,
    ) {
        let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
            &self.runtime.owned.durable_state_store,
            "local",
        );
        let trigger = self.app_trigger(&catalog).await;
        let receipt = self.emit_app(catalog.clone(), &trigger, "unsettled");
        self.runtime
            .queue_agent_notification_input(
                &self.session_id,
                trigger.id(),
                &receipt.receipt_id,
                serde_json::json!({"installation_id":catalog.installation_id(),"event_name":"changed","occurrence_id":receipt.occurrence_id,"occurred_at_ms":receipt.occurred_at_ms,"deadline_ms":receipt.expires_at_ms,"payload":{"text":"unsettled"}}),
            )
            .await
            .unwrap()
            .expect("the App notification prompt is admitted");
        (catalog, receipt)
    }

    /// What happens to an admitted notification prompt before its receipt settles.
    async fn end_the_admitted_notification(&self, outcome: &str, prompt: &str) {
        match outcome {
            "queued" => {}
            "completed" => {
                // The provider turn-end settlement: the running turn ends and the
                // notification starts, then the notification's own turn ends.
                let room = self
                    .runtime
                    .owned
                    .session_snapshot(&self.session_id)
                    .unwrap();
                let (active, queued) = self
                    .runtime
                    .owned
                    .prompt_state_owner
                    .state_parts(&room, &self.agent_id);
                let running = active.unwrap().id().to_owned();
                let next = queued
                    .iter()
                    .find(|item| item.id() == prompt)
                    .unwrap()
                    .clone();
                let started = self
                    .runtime
                    .complete_local_prompt_with_queued_advance_if_matches(
                        &self.session_id,
                        &self.agent_id,
                        None,
                        &next,
                        Some(&running),
                    )
                    .await
                    .unwrap()
                    .expect("the running turn completes");
                assert_eq!(
                    started
                        .completion
                        .started_next
                        .map(|item| item.id().to_owned()),
                    Some(prompt.to_owned())
                );
                self.runtime
                    .owned
                    .complete_local_prompt_without_advance_if_matches(
                        &self.session_id,
                        &self.agent_id,
                        None,
                        Some(prompt),
                    )
                    .unwrap()
                    .expect("the notification turn completes");
            }
            "steered" => {
                self.runtime
                    .owned
                    .steer_queued_prompt(
                        &self.session_id,
                        &self.agent_id,
                        &self.attachment_id,
                        prompt,
                    )
                    .unwrap();
            }
            "cancelled" => {
                self.runtime
                    .owned
                    .cancel_queued_prompt(
                        &self.session_id,
                        &self.agent_id,
                        &self.attachment_id,
                        prompt,
                    )
                    .unwrap();
            }
            other => unreachable!("{other}"),
        }
        assert!(
            outcome == "queued" || self.queued_prompts().is_empty(),
            "{outcome}"
        );
    }

    /// State and prompt of every agent notification receipt.
    fn notification_receipts(&self) -> Vec<(String, Option<String>)> {
        let db = rusqlite::Connection::open(self.runtime.owned.durable_state_store.path()).unwrap();
        let mut query = db
            .prepare("SELECT state,queued_prompt_id FROM app_outbox WHERE source_kind IN ('workflow_completion','app_event') ORDER BY receipt_id")
            .unwrap();
        query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// The provider takes the admitted notification prompt as its next turn.
    fn start_the_queued_notification(&self) {
        let room = self
            .runtime
            .owned
            .session_store
            .get_session(&self.session_id)
            .unwrap();
        let state = &self.runtime.owned.prompt_state_owner;
        state
            .complete_active_prompt_only(&room, &self.agent_id)
            .unwrap();
        state
            .activate_next_queued_prompt(&room, &self.agent_id, None)
            .unwrap();
    }

    /// The agent invokes `workflow` from its active turn; the completion must not notify it back.
    async fn assert_the_cycle_is_dropped(&self, workflow: &str) {
        let invocation = self
            .runtime
            .owned
            .inherit_agent_notification_ancestry(Some(&self.agent_id), "entry", None)
            .unwrap();
        self.complete_with_invocation(workflow, "cycle", "done", invocation);
        self.pass().await;
        assert!(
            self.queued_prompts().is_empty(),
            "A -> W -> A must be dropped"
        );
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: a fast provider may invoke a workflow before the pump settles the receipt.
#[tokio::test]
async fn a_workflow_invoked_before_the_receipt_settles_keeps_the_notification_lineage() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.complete(&workflow, "first", "released");
    rig.admit_workflow_notification_unsettled().await;
    rig.start_the_queued_notification();
    let lineage = rig.lineage_of_the_active_agent();
    assert_eq!(lineage.last().unwrap(), &format!("agent:{}", rig.agent_id));
    let room = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    assert!(lineage.contains(&store::workflow_identity(
        room.host_daemon_id(),
        &rig.session_id,
        &workflow
    )));
    rig.assert_the_cycle_is_dropped(&workflow).await;
}

// MP-08 / MP-09 / MP-10 / MP-11: a crash between admission and settlement keeps the admitted prompt's lineage.
#[tokio::test]
async fn a_restart_between_admission_and_settlement_keeps_the_notification_lineage() {
    for app in [false, true] {
        let config = super::super::agent_trigger_runtime_state::tests::durable_state_config(
            "n2-lineage-recovery",
        );
        let (root, runtime, session_id, agent_id, attachment_id, _, _) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config.clone(),
                &[],
            )
            .await;
        let mut rig = Rig {
            _root: root,
            runtime,
            session_id,
            agent_id,
            attachment_id,
        };
        let workflow = rig.workflow_with_exit("recovery-pipeline");
        let source = rig.register(&workflow);
        if app {
            rig.admit_app_notification_unsettled().await;
        } else {
            rig.runtime
                .create_agent_trigger(rig.request(&source), "local")
                .await
                .unwrap();
            rig.complete(&workflow, "first", "released");
            rig.admit_workflow_notification_unsettled().await;
        }
        let mut restart_config = config.clone();
        let restart_path = super::super::agent_trigger_runtime_state::tests::durable_state_config(
            "n2-lineage-recovery-restart",
        )
        .user_config
        .state
        .path
        .unwrap();
        rusqlite::Connection::open(rig.runtime.owned.durable_state_store.path())
            .unwrap()
            .execute("VACUUM INTO ?1", rusqlite::params![restart_path])
            .unwrap();
        restart_config.user_config.state.path = Some(restart_path);
        let restarted = crate::app::DaemonApp::bootstrap(restart_config).unwrap();
        rig.runtime = super::super::local_prompt_dispatch_runtime::tests::owned_runtime_state(
            &std::sync::Arc::new(tokio::sync::Mutex::new(restarted)),
        )
        .await;
        assert_eq!(
            rig.queued_prompts().len(),
            1,
            "recovery restores the admitted notification prompt"
        );
        rig.start_the_queued_notification();
        assert_eq!(
            rig.lineage_of_the_active_agent().last().unwrap(),
            &format!("agent:{}", rig.agent_id),
            "app: {app}"
        );
        if app {
            rig.runtime
                .create_agent_trigger(rig.request(&source), "local")
                .await
                .unwrap();
        }
        rig.assert_the_cycle_is_dropped(&workflow).await;
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: a receipt that names its admitted prompt is settled from it, never fired again.
#[tokio::test]
async fn an_admitted_receipt_is_settled_from_its_prompt_after_a_restart() {
    for outcome in ["queued", "completed", "steered", "cancelled"] {
        let config = super::super::agent_trigger_runtime_state::tests::durable_state_config(
            "n1-admission-fence",
        );
        let (root, runtime, session_id, agent_id, attachment_id, _, _) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
                config.clone(),
                &[],
            )
            .await;
        let mut rig = Rig {
            _root: root,
            runtime,
            session_id,
            agent_id,
            attachment_id,
        };
        let workflow = rig.workflow_with_exit("fence-pipeline");
        let source = rig.register(&workflow);
        rig.runtime
            .create_agent_trigger(rig.request(&source), "local")
            .await
            .unwrap();
        rig.complete(&workflow, "first", "released");
        let (_, _, prompt) = rig.admit_workflow_notification_unsettled().await;
        rig.end_the_admitted_notification(outcome, &prompt).await;
        let mut restart_config = config.clone();
        let restart_path = super::super::agent_trigger_runtime_state::tests::durable_state_config(
            "n1-admission-fence-restart",
        )
        .user_config
        .state
        .path
        .unwrap();
        rusqlite::Connection::open(rig.runtime.owned.durable_state_store.path())
            .unwrap()
            .execute("VACUUM INTO ?1", rusqlite::params![restart_path])
            .unwrap();
        restart_config.user_config.state.path = Some(restart_path);
        let restarted = crate::app::DaemonApp::bootstrap(restart_config).unwrap();
        rig.runtime = super::super::local_prompt_dispatch_runtime::tests::owned_runtime_state(
            &std::sync::Arc::new(tokio::sync::Mutex::new(restarted)),
        )
        .await;
        rig.pass().await;
        assert_eq!(
            rig.queued_prompts().len(),
            usize::from(outcome == "queued"),
            "no second prompt for the occurrence ({outcome}): {:?} {:?}",
            rig.queued_prompts(),
            rig.notification_receipts()
        );
        let receipts = rig.notification_receipts();
        assert_eq!(receipts.len(), 1, "{outcome}");
        assert!(
            !["accepted", "retryable"].contains(&receipts[0].0.as_str())
                && receipts[0].1.as_deref() == Some(prompt.as_str()),
            "the receipt is settled from its admitted prompt ({outcome}): {receipts:?}"
        );
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: the App router settles an admitted receipt from its prompt.
#[tokio::test]
async fn an_admitted_app_receipt_is_settled_from_its_prompt() {
    use chariox_app_runtime::app_outbox::ReceiptState;
    for outcome in ["queued", "completed", "steered", "cancelled"] {
        let rig = rig().await;
        let (catalog, receipt) = rig.admit_app_notification_unsettled().await;
        let store = &rig.runtime.owned.durable_state_store;
        let prompt = store
            .admitted_agent_notification_prompt(&receipt.receipt_id)
            .unwrap()
            .expect("admission names the prompt");
        rig.end_the_admitted_notification(outcome, &prompt).await;
        rig.runtime
            .route_app_agent_installation("local", catalog.clone())
            .await
            .unwrap();
        assert_eq!(
            rig.queued_prompts().len(),
            usize::from(outcome == "queued"),
            "no second prompt for the occurrence ({outcome})"
        );
        let settled = store
            .app_event_candidate("local", catalog, &receipt.receipt_id)
            .unwrap()
            .receipt()
            .clone();
        assert_ne!(settled.state, ReceiptState::Accepted, "{outcome}");
        assert_ne!(settled.state, ReceiptState::Retryable, "{outcome}");
        assert_eq!(settled.queued_prompt_id, Some(prompt), "{outcome}");
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: an App notification is in the lineage before the router settles its receipt.
#[tokio::test]
async fn an_app_notified_workflow_invoked_before_the_receipt_settles_drops_the_cycle() {
    let rig = rig().await;
    rig.admit_app_notification_unsettled().await;
    rig.start_the_queued_notification();
    assert_eq!(
        rig.lineage_of_the_active_agent(),
        vec![format!("agent:{}", rig.agent_id)]
    );
    let workflow = rig.workflow_with_exit("reply");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.assert_the_cycle_is_dropped(&workflow).await;
}

// MP-08 / MP-09 / MP-10 / MP-11: the commit that releases a rejected Steer's queue hold also ends its lineage.
#[tokio::test]
async fn a_persisted_steer_rejection_survives_recovery_without_its_provisional_lineage() {
    let config =
        super::super::agent_trigger_runtime_state::tests::durable_state_config("n3b-rejection");
    let (root, runtime, session_id, agent_id, attachment_id, _, _) =
        super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config(
            config.clone(),
            &[],
        )
        .await;
    let mut rig = Rig {
        _root: root,
        runtime,
        session_id,
        agent_id,
        attachment_id,
    };
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.complete(&workflow, "first", "released");
    rig.pass().await;
    let room = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    let notification = rig
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&room, &rig.agent_id)
        .1[0]
        .id()
        .to_owned();
    let (prepared, reservation) = rig.remote_steer_intent(&notification);
    let target = prepared.target_active_prompt_id.clone();
    // Only the durable rejection lands; nothing runs after it, as in a crash.
    let (active, queued) = rig
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&room, &rig.agent_id);
    queued[0]
        .clear_remote_steer_outcome_uncertainty(reservation)
        .expect("the reserved uncertain send");
    rig.runtime
        .owned
        .mirror_prompt_owner_agent_state(&rig.session_id, &rig.agent_id, active, queued)
        .unwrap();
    let mut restart_config = config.clone();
    let restart_path = super::super::agent_trigger_runtime_state::tests::durable_state_config(
        "n3b-rejection-restart",
    )
    .user_config
    .state
    .path
    .unwrap();
    rusqlite::Connection::open(rig.runtime.owned.durable_state_store.path())
        .unwrap()
        .execute("VACUUM INTO ?1", rusqlite::params![restart_path])
        .unwrap();
    restart_config.user_config.state.path = Some(restart_path);
    let restarted = crate::app::DaemonApp::bootstrap(restart_config).unwrap();
    rig.runtime = super::super::local_prompt_dispatch_runtime::tests::owned_runtime_state(
        &std::sync::Arc::new(tokio::sync::Mutex::new(restarted)),
    )
    .await;
    assert!(
        rig.runtime
            .owned
            .durable_state_store
            .agent_notification_ancestry(&rig.session_id, &target)
            .unwrap()
            .is_none(),
        "the original turn never received the rejected input"
    );
    rig.complete(&workflow, "after-rejection", "released again");
    rig.pass().await;
    assert_eq!(
        rig.queued_prompts().len(),
        2,
        "a workflow the original turn invoked still notifies"
    );
}

#[tokio::test]
async fn a_remote_agent_follow_the_source_does_not_confirm_is_reconciled_as_absent() {
    let rig = rig().await;
    let owner = "local";
    let home = rig.runtime.owned.config_projection.snapshot().daemon_id;
    let subscription = WorkflowNotificationSubscription {
        subscription_id: "sub-unconfirmed".into(),
        source_id: "wf_source_remote".into(),
        owner_user_id: owner.into(),
        created_by_user_id: owner.into(),
        target_kernel_id: home.clone(),
        target_kind: WorkflowNotificationTargetKind::Agent,
        session_id: rig.session_id.clone(),
        workflow_id: String::new(),
        publication_id: "trigger-unconfirmed".into(),
        endpoint_id: rig.agent_id.clone(),
        queue_id: AGENT_QUEUE_ID.into(),
        source_available: true,
        source_kernel_id: "remote-kernel".into(),
        agent_id: Some(rig.agent_id.clone()),
        trigger_id: Some("trigger-unconfirmed".into()),
    };
    assert!(rig
        .runtime
        .subscribe_agent_notification(&home, subscription)
        .await
        .is_err());
    let (_, active, _) = rig
        .runtime
        .owned
        .durable_state_store
        .notification_inventory(owner)
        .unwrap();
    assert!(active.is_empty(), "the refused follow stays active");
    let crate::durable_state::workflow_notifications::NotificationOutcome::Reconciles(due) = rig
        .runtime
        .owned
        .durable_state_store
        .notify(NotificationOperation::ClaimRemoteReconciles {
            now: 1_000,
            limit: 8,
        })
        .unwrap()
    else {
        panic!("a claim answers with the due reconciliations");
    };
    assert_eq!(
        due.len(),
        1,
        "the source may hold a row nothing here knows of"
    );
    assert!(due[0].desired.is_none());
    assert_eq!(due[0].source_kernel_id, "remote-kernel");
}

// MP-08 / MP-09 / MP-10 / MP-11 (W1): a thread that holds the session store and checks an agent
// notification target must finish while another thread waits to write. The std lock queues a
// reader behind a waiting writer, so a second read taken under the first never returns.
#[tokio::test]
async fn the_agent_target_check_under_a_held_session_read_completes_while_a_writer_waits() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    let subscription = rig.subscriptions().remove(0);
    let owned = rig.runtime.owned.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (go_tx, go_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let reader = {
        let owned = owned.clone();
        std::thread::spawn(move || {
            let sessions = owned.session_store.read();
            ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            let live = sessions
                .get_session(&subscription.session_id)
                .is_ok_and(|session| owned.agent_notification_target_live(&session, &subscription));
            drop(sessions);
            done_tx.send(live).unwrap();
        })
    };
    ready_rx.recv().unwrap();
    let writer = {
        let owned = owned.clone();
        std::thread::spawn(move || drop(owned.session_store.write()))
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    go_tx.send(()).unwrap();
    let live = done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the target check re-locked the session store under a waiting writer");
    assert!(live);
    reader.join().unwrap();
    writer.join().unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11 (W1): accepting a local agent notification while the session
// store is written all the time (the output pump settling runs) never wedges the kernel.
#[tokio::test]
async fn accepting_agent_notifications_while_the_session_store_is_written_never_blocks() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    let subscription = rig.subscriptions().remove(0);
    let envelope = crate::local::WorkflowNotificationEnvelope {
        source_id: source,
        display_name: "pipeline".into(),
        occurrence_id: "occurrence-w1".into(),
        output: WorkflowOutputPayload::new("release approved", vec![]),
        ancestry: vec![],
        deadline_ms: crate::session::unix_epoch_ms() + 3_600_000,
    };
    let owned = rig.runtime.owned.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let (owned, stop) = (owned.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                drop(owned.session_store.write());
                std::thread::yield_now();
            }
        })
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..2_000 {
            owned.accept_local_agent_notification(&subscription, &envelope);
        }
        done_tx.send(()).unwrap();
    });
    let finished = done_rx.recv_timeout(std::time::Duration::from_secs(30));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    finished.expect("accepting an agent notification waited on the session store for ever");
    writer.join().unwrap();
}

impl Rig {
    /// `/workflow trigger notification list ... detach <subscription-id>` of the one listener.
    async fn detach_subscription(&self) {
        let subscription = self.subscriptions().remove(0);
        let response = self
            .runtime
            .execute_workflow_notification_command(
                LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {
                    session_id: self.session_id.clone(),
                    subscription_id: subscription.subscription_id.clone(),
                }),
                "local",
            )
            .await
            .unwrap();
        assert!(matches!(
            response,
            LocalDaemonResponse::WorkflowNotificationDetached { subscription_id }
                if subscription_id == subscription.subscription_id
        ));
    }
}

// MP-08 / MP-09 / MP-10 / MP-11 (N1): the trigger is the agent's listener, so detaching its
// subscription from the notification list removes the trigger with it.
#[tokio::test]
async fn detaching_an_agent_notification_subscription_removes_its_trigger() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();

    rig.detach_subscription().await;

    let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    assert!(
        session.agent_triggers().is_empty(),
        "the trigger outlived its subscription"
    );
    assert!(rig.subscriptions().is_empty());
}

// MP-08 / MP-09 / MP-10 / MP-11 (N1): subscribing again after a detach listens again.
#[tokio::test]
async fn subscribing_again_after_a_detach_queues_the_next_occurrence() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    rig.detach_subscription().await;

    rig.runtime
        .create_agent_trigger(rig.request(&source), "local")
        .await
        .unwrap();
    assert_eq!(rig.subscriptions().len(), 1);
    rig.complete(&workflow, "run-0", "release 0 approved");
    rig.pass().await;

    let queued = rig.queued_prompts();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(queued[0].contains("release 0 approved"), "{queued:?}");
}

// MP-08 / MP-10: turning a local source off or on updates the trigger that listens to it.
#[tokio::test]
async fn a_notification_trigger_follows_its_local_source_enabled_state() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    let trigger = rig.trigger_on(&source).await;
    rig.runtime
        .sync_agent_notification_trigger_availability()
        .unwrap();
    assert!(rig.trigger_listening(&trigger));

    rig.set_source_enabled(&workflow, false).await;
    assert!(
        !rig.trigger_listening(&trigger),
        "a disabled source no longer reaches the trigger"
    );
    rig.set_source_enabled(&workflow, true).await;
    assert!(rig.trigger_listening(&trigger), "the trigger recovers");
}

// MP-08 / MP-10: a workflow whose exit stops completing runs no longer feeds its trigger.
#[tokio::test]
async fn a_notification_trigger_follows_the_removal_of_its_local_exit() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    let trigger = rig.trigger_on(&source).await;
    rig.runtime
        .sync_agent_notification_trigger_availability()
        .unwrap();
    assert!(rig.trigger_listening(&trigger));

    let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    let exit = session
        .workflow(&workflow)
        .unwrap()
        .nodes()
        .iter()
        .find(|node| node.can_complete_workflow_run())
        .unwrap()
        .id()
        .to_owned();
    let (result, _) = rig
        .runtime
        .execute_workflow_request(
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
                crate::local::SetWorkflowNodeCanCompleteRunRequest {
                    session_id: rig.session_id.clone(),
                    workflow_ref: workflow.clone(),
                    node_id: exit,
                    can_complete_workflow_run: false,
                    expected_workflow_revision: None,
                },
            ),
            "local".into(),
            None,
        )
        .await;
    result.unwrap();
    assert!(
        !rig.trigger_listening(&trigger),
        "an exit that is gone leaves the trigger unavailable"
    );
}

// MP-08 / MP-10: the projection a workflow command publishes, and the response it returns, carry
// the availability the command just changed.
#[tokio::test]
async fn the_workflow_actor_publishes_the_trigger_availability_its_command_changed() {
    let rig = rig().await;
    let workflow = rig.workflow_with_exit("pipeline");
    let source = rig.register(&workflow);
    let trigger = rig.trigger_on(&source).await;
    rig.runtime
        .sync_agent_notification_trigger_availability()
        .unwrap();
    assert!(rig.trigger_listening(&trigger));
    let session = rig.runtime.owned.session_snapshot(&rig.session_id).unwrap();
    let exit = session
        .workflow(&workflow)
        .unwrap()
        .nodes()
        .iter()
        .find(|node| node.can_complete_workflow_run())
        .unwrap()
        .id()
        .to_owned();

    let actor = crate::runtime::workflow_actor::WorkflowRuntime::new(
        rig.runtime.clone(),
        rig.runtime.owned.session_projection.clone(),
        rig.runtime.owned.agent_runtime_projection.clone(),
    );
    let request = LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
        crate::local::SetWorkflowNodeCanCompleteRunRequest {
            session_id: rig.session_id.clone(),
            workflow_ref: workflow.clone(),
            node_id: exit,
            can_complete_workflow_run: false,
            expected_workflow_revision: None,
        },
    );
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "workflow-actor",
        None,
        None,
        &request,
    );
    let response = actor
        .dispatch_workflow_command(command, request)
        .await
        .unwrap();

    let LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. } = response else {
        panic!("wrong response")
    };
    assert!(
        !trigger_listening_in(&session, &trigger),
        "the response still shows the trigger available"
    );
    let published = rig
        .runtime
        .owned
        .session_projection
        .get(&rig.session_id)
        .unwrap();
    assert!(
        !trigger_listening_in(&published, &trigger),
        "the published projection still shows the trigger available"
    );
}
