//! A workflow's responsible agent on another kernel: its trigger lives there, a failed run
//! reaches it through the existing notification path, a hand-over detaches it, and its
//! deletion (or its kernel's removal from the account) gives the workflow back to the person.
use super::availability::{on_a_big_stack, Drill};
use crate::local::WorkflowResponsibleTarget;
use crate::session::{
    ResponsibleNoticeKind, WorkflowFailureEvent, WorkflowFailureKind, WorkflowResponsible,
    WorkflowRunStatus,
};

const AGENT: &str = "agent";

/// The target kernel's agent is a live agent there, as a Room agent is.
fn register_agent(drill: &Drill) {
    let agent = drill
        .target_runtime
        .owned
        .session_store
        .get_session(&drill.target.session)
        .unwrap()
        .agents()
        .iter()
        .find(|agent| agent.id() == AGENT)
        .unwrap()
        .clone();
    drill.target_runtime.owned.agent_store.restore_agent(agent);
    // A turn is running: what the trigger fires queues behind it, as it does for a busy agent.
    drill
        .target_runtime
        .owned
        .provider_store
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                &drill.target.session,
                "dev-stub",
                "dev-stub",
                "default",
                "room-model",
            )
            .with_agent_id(AGENT),
        )
        .unwrap();
}

fn target_of(drill: &Drill) -> WorkflowResponsibleTarget {
    WorkflowResponsibleTarget::Agent {
        kernel_id: drill.target_runtime.owned.home_kernel_id(),
        room_id: drill.target.session.clone(),
        agent_id: AGENT.into(),
    }
}

/// The workflow as the source kernel holds it, with its Room.
fn workflow_of(
    drill: &Drill,
) -> (
    crate::session::RuntimeSession,
    crate::session::WorkflowDefinition,
) {
    let room = drill
        .source_runtime
        .owned
        .session_store
        .get_session(&drill.source.session)
        .unwrap();
    let workflow = room.workflow(&drill.workflow).unwrap().clone();
    (room, workflow)
}

async fn set(drill: &Drill, target: WorkflowResponsibleTarget) {
    let (room, workflow) = workflow_of(drill);
    drill
        .source_runtime
        .change_workflow_responsible(&room, &workflow, target, None, true)
        .await
        .unwrap();
}

fn responsible_triggers(drill: &Drill) -> usize {
    drill
        .target_runtime
        .owned
        .session_store
        .get_session(&drill.target.session)
        .unwrap()
        .agent_triggers()
        .iter()
        .filter(|trigger| trigger.responsible_for().is_some())
        .count()
}

/// A run that failed on the source kernel, saved the way the kernel saves it.
fn fail_run(drill: &Drill, id: &str, message: &str) {
    let workflow = drill.workflow.clone();
    let mut run = crate::session::WorkflowRun::new(
        id,
        workflow.clone(),
        "endpoint",
        "node",
        Some("secret input of the run".into()),
        None,
        vec![],
        vec![],
    );
    run.add_failure_event(WorkflowFailureEvent::new(
        WorkflowFailureKind::StepNeedsHelp,
        "node-run",
        vec![],
        message,
    ));
    run.set_status(WorkflowRunStatus::Failed);
    let mut session = drill
        .source_runtime
        .owned
        .session_store
        .get_session(&drill.source.session)
        .unwrap();
    session.create_workflow_run(run);
    drill
        .source_runtime
        .owned
        .session_store
        .write()
        .restore_session(session.clone());
    drill
        .source
        .store
        .persist_workflow_runtime_transition(&session, "a run failed")
        .unwrap();
}

/// What the agent's triggers say, for a failure message.
fn trigger_report(drill: &Drill) -> String {
    let room = drill
        .target_runtime
        .owned
        .session_store
        .get_session(&drill.target.session)
        .unwrap();
    room.agent_triggers()
        .iter()
        .map(|trigger| {
            format!(
                "{} fired={} error={:?} source={:?}",
                trigger.id(),
                trigger.fired(),
                trigger.last_error(),
                trigger.source()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn prompts_of_agent(drill: &Drill) -> Vec<String> {
    let room = drill
        .target_runtime
        .owned
        .session_store
        .get_session(&drill.target.session)
        .unwrap();
    room.active_prompt_for_agent(AGENT)
        .into_iter()
        .chain(room.queued_prompts_for_agent(AGENT).into_iter().flatten())
        .map(|prompt| prompt.prompt().to_owned())
        .collect()
}

async fn pump(drill: &Drill, until: impl Fn(&Drill) -> bool) {
    for _ in 0..40 {
        for runtime in [&drill.source_runtime, &drill.target_runtime] {
            runtime.route_remote_workflow_notifications().await;
            runtime.owned.route_workflow_notifications();
            runtime.route_agent_notifications().await;
        }
        if until(drill) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[test]
fn an_agent_on_another_kernel_is_told_of_a_failed_run_and_released_by_a_hand_over() {
    on_a_big_stack(async {
        let drill = Drill::connect().await;
        register_agent(&drill);
        set(&drill, target_of(&drill)).await;

        let (_, workflow) = workflow_of(&drill);
        assert_eq!(
            workflow.responsible().agent().map(|agent| agent.agent_id),
            Some(AGENT),
            "the workflow's kernel records the agent of the other kernel"
        );
        assert_eq!(
            responsible_triggers(&drill),
            1,
            "its trigger lives on its own kernel"
        );
        let subscriptions = drill
            .source
            .store
            .notification_inventory("local")
            .unwrap()
            .1;
        assert!(
            subscriptions
                .iter()
                .any(|s| s.subscription_id.starts_with("wf_responsible_")
                    && s.source_kernel_id == drill.source_runtime.owned.home_kernel_id()),
            "{subscriptions:?}"
        );

        fail_run(&drill, "run-1", "resend the diff");
        pump(&drill, |drill| !prompts_of_agent(drill).is_empty()).await;
        let prompts = prompts_of_agent(&drill);
        assert_eq!(prompts.len(), 1, "{prompts:?} {}", trigger_report(&drill));
        assert!(
            prompts[0]
                .starts_with("reviewer · Run #1 failed — a step needs help: resend the diff."),
            "{}",
            prompts[0]
        );
        assert!(!prompts[0].contains("secret input"), "{}", prompts[0]);

        // Handing the workflow back detaches the agent's trigger on its kernel, and that is not
        // the agent going away: the workflow stays the person's with no notice.
        set(&drill, WorkflowResponsibleTarget::Person).await;
        assert_eq!(responsible_triggers(&drill), 0);
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let (_, workflow) = workflow_of(&drill);
        assert_eq!(workflow.responsible(), &WorkflowResponsible::Person);
        assert!(workflow.responsible_notice().is_none());
        drill.finish().await;
    });
}

#[test]
fn an_agent_of_another_kernel_takes_a_workflow_through_its_own_kernel() {
    on_a_big_stack(async {
        let drill = Drill::connect().await;
        register_agent(&drill);
        let target_kernel = drill.target_runtime.owned.home_kernel_id();
        let source_kernel = drill.source_runtime.owned.home_kernel_id();
        let sender = crate::transport::relay_peer::PeerAgentStamp {
            kernel_id: target_kernel.clone(),
            room_id: drill.target.session.clone(),
            agent_id: AGENT.into(),
        };
        let request =
            |responsible| crate::transport::relay_peer::RelayPeerRequest::SetWorkflowResponsible {
                protocol_version: crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                sender: sender.clone(),
                room: drill.source.session.clone(),
                workflow_ref: drill.workflow.clone(),
                responsible,
            };
        let respond = |responsible| async {
            drill
                .target_runtime
                .peer_agent_link("local", &source_kernel, request(responsible))
                .await
        };
        // A read answers with the workflow as it is.
        let crate::transport::relay_peer::RelayPeerResponse::WorkflowResponsibleChanged {
            workflow,
            ..
        } = respond(None).await.unwrap()
        else {
            panic!("a read answers the workflow");
        };
        assert_eq!(workflow.responsible(), &WorkflowResponsible::Person);

        // The agent takes it for itself; its own kernel is then asked to keep the trigger.
        let crate::transport::relay_peer::RelayPeerResponse::WorkflowResponsibleChanged {
            workflow,
            ..
        } = respond(Some(target_of(&drill))).await.unwrap()
        else {
            panic!("a take answers the workflow");
        };
        assert_eq!(
            workflow.responsible().agent().map(|a| a.agent_id),
            Some(AGENT)
        );
        assert_eq!(responsible_triggers(&drill), 1);

        // A stranger agent may not hand it over.
        let stranger = crate::transport::relay_peer::PeerAgentStamp {
            kernel_id: target_kernel,
            room_id: drill.target.session.clone(),
            agent_id: "someone-else".into(),
        };
        let refused = drill
            .target_runtime
            .peer_agent_link(
                "local",
                &source_kernel,
                crate::transport::relay_peer::RelayPeerRequest::SetWorkflowResponsible {
                    protocol_version: crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    sender: stranger,
                    room: drill.source.session.clone(),
                    workflow_ref: drill.workflow.clone(),
                    responsible: Some(WorkflowResponsibleTarget::Person),
                },
            )
            .await;
        assert!(refused.is_err(), "{refused:?}");
        drill.finish().await;
    });
}

#[test]
fn a_deleted_responsible_agent_gives_the_workflow_back_with_a_notice() {
    on_a_big_stack(async {
        let drill = Drill::connect().await;
        register_agent(&drill);
        set(&drill, target_of(&drill)).await;
        assert_eq!(responsible_triggers(&drill), 1);

        // The agent is deleted on its kernel: what fires prompts into it goes with it, and the
        // kernel's own pass ends the subscription the trigger held.
        drill
            .target_runtime
            .owned
            .session_store
            .write()
            .remove_agent_triggers_for_agent(&drill.target.session, AGENT)
            .unwrap();
        pump(&drill, |drill| {
            workflow_of(drill).1.responsible().is_person()
        })
        .await;
        let (_, workflow) = workflow_of(&drill);
        assert_eq!(workflow.responsible(), &WorkflowResponsible::Person);
        assert_eq!(
            workflow
                .responsible_notice()
                .map(|notice| (notice.kind, notice.label.as_str())),
            Some((ResponsibleNoticeKind::AgentGone, "agent-ref")),
            "the person is told once which agent was removed"
        );
        drill.finish().await;
    });
}

#[test]
fn a_kernel_removed_from_the_account_gives_its_workflows_back_but_an_offline_one_keeps_them() {
    on_a_big_stack(async {
        let drill = Drill::connect().await;
        register_agent(&drill);
        set(&drill, target_of(&drill)).await;
        let target_kernel = drill.target_runtime.owned.home_kernel_id();
        let projection = &drill.source_runtime.owned.notification_inventory_projection;

        // Offline is still linked to the account: two reads later nothing has changed.
        projection.record_directory_read([target_kernel.clone()].into());
        projection.record_directory_read([target_kernel.clone()].into());
        drill.source_runtime.fall_back_for_gone_kernels().await;
        assert!(!workflow_of(&drill).1.responsible().is_person());

        // One read without it is not enough; the second makes it gone for good.
        projection.record_directory_read(Default::default());
        drill.source_runtime.fall_back_for_gone_kernels().await;
        assert!(!workflow_of(&drill).1.responsible().is_person());
        projection.record_directory_read(Default::default());
        drill.source_runtime.fall_back_for_gone_kernels().await;
        let (_, workflow) = workflow_of(&drill);
        assert_eq!(workflow.responsible(), &WorkflowResponsible::Person);
        assert_eq!(
            workflow.responsible_notice().map(|notice| notice.kind),
            Some(ResponsibleNoticeKind::MachineGone)
        );
        drill.finish().await;
    });
}
