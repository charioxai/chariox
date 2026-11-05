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
        let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = response else {
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
        assert!(text.contains("\"display_name\":\"pipeline\""), "{text}");
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
    let payload =
        serde_json::json!({"source_id":source,"occurrence_id":"receipt-retry","output":"released"});
    let first = rig
        .runtime
        .queue_agent_notification_input(&rig.session_id, trigger.id(), payload.clone())
        .await
        .unwrap()
        .unwrap();
    let second = rig
        .runtime
        .queue_agent_notification_input(&rig.session_id, trigger.id(), payload)
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
