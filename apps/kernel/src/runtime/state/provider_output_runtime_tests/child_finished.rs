//! The parent always follows its children: the end of a child's work is one urgent
//! `child_finished` event that steers a running parent and wakes a parked one.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

pub(super) struct Kid {
    pub(super) child: String,
    pub(super) child_prompt: crate::session::PromptQueueItem,
    pub(super) child_run: String,
    pub(super) task: crate::durable_state::agent_lifecycle::AgentTaskExecution,
    pub(super) correction: bool,
}

pub(super) struct Family {
    pub(super) runtime: KernelRuntimeState,
    pub(super) room: String,
    pub(super) parent: String,
    pub(super) parent_run: String,
    pub(super) kids: Vec<Kid>,
}

pub(super) struct Team {
    pub(super) runtime: KernelRuntimeState,
    pub(super) room: String,
    pub(super) parent: String,
    pub(super) child: String,
    pub(super) child_prompt: crate::session::PromptQueueItem,
    pub(super) child_run: String,
    pub(super) task: crate::durable_state::agent_lifecycle::AgentTaskExecution,
    pub(super) correction: bool,
}

/// A parent and its child in one room; the child has an active turn and a task, not yet settled.
pub(super) async fn team(label: &str, settle_cancelled: bool, parent_running: bool) -> Team {
    team_with(label, Some(settle_cancelled), parent_running).await
}

/// `settle_cancelled: None` leaves the child working.
pub(super) async fn team_with(
    label: &str,
    settle_cancelled: Option<bool>,
    parent_running: bool,
) -> Team {
    let Family {
        runtime,
        room,
        parent,
        mut kids,
        ..
    } = family(label, 1, settle_cancelled, parent_running).await;
    let Kid {
        child,
        child_prompt,
        child_run,
        task,
        correction,
    } = kids.remove(0);
    Team {
        runtime,
        room,
        parent,
        child,
        child_prompt,
        child_run,
        task,
        correction,
    }
}

/// A parent and `children` sub-agents, each with an active turn and a task.
pub(super) async fn family(
    label: &str,
    children: usize,
    settle_cancelled: Option<bool>,
    parent_running: bool,
) -> Family {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    // Spawning a child needs the parent's live turn.
    crate::test_support::admit_room_test_turn(&mut app, session.id(), parent.id());
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "team-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut spawned = Vec::new();
    for index in 0..children {
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias(if index == 0 {
                        "child".to_owned()
                    } else {
                        format!("child{}", index + 1)
                    })
                    .with_parent_agent_id(parent.id()),
            )
            .unwrap();
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(child.id()),
            )
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(child.id()),
                "do the work",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("the child starts its prompt")
        };
        spawned.push((child.id().to_owned(), run.id().to_owned(), prompt));
    }
    let parent_run = app
        .providers()
        .get_run_for_agent(session.id(), parent.id())
        .expect("the admitted turn has a run");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let store = &runtime.owned.durable_state_store;
    let mut kids = Vec::new();
    for (child, child_run, prompt) in spawned {
        let Outcome::Task(begun) = store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: session.id().into(),
                agent: child.clone(),
                prompt: prompt.id().into(),
                run: Some(child_run.clone()),
                now: 1,
            })
            .unwrap()
        else {
            panic!("begun")
        };
        let (task, correction) = match settle_cancelled {
            None => (begun, false),
            Some(cancelled) => {
                let Outcome::Settled { task, correction } = store
                    .agent_lifecycle(Operation::Settle {
                        room: session.id().into(),
                        agent: child.clone(),
                        prompt: prompt.id().into(),
                        run: child_run.clone(),
                        has_answer: !cancelled,
                        cancelled,
                        now: 2,
                    })
                    .unwrap()
                else {
                    panic!("settled")
                };
                runtime
                    .owned
                    .complete_local_prompt_without_advance(session.id(), &child, Some(&child_run))
                    .unwrap();
                (task, correction)
            }
        };
        kids.push(Kid {
            child,
            child_prompt: prompt,
            child_run,
            task,
            correction,
        });
    }
    if !parent_running {
        // The parent ended its turn and parked on its children.
        runtime
            .owned
            .complete_local_prompt_without_advance(session.id(), parent.id(), Some(parent_run.id()))
            .unwrap();
    }
    Family {
        room: session.id().into(),
        parent: parent.id().into(),
        parent_run: parent_run.id().into(),
        kids,
        runtime,
    }
}

fn child_finished(team: &Team) -> Vec<crate::durable_state::agent_lifecycle::InboxEvent> {
    team.runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "child_finished")
        .collect()
}

#[tokio::test]
async fn a_finished_child_is_one_urgent_event_for_its_parent() {
    let team = team("child-finished-urgent", false, false).await;
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert!(event.urgent, "a finish steers the parent");
    assert_eq!(event.source_id, team.child);
    assert_eq!(event.payload["child_agent_id"], team.child.as_str());
    assert_eq!(event.payload["success"], true);
    assert_eq!(event.payload["outcome"]["kind"], "completed");
    assert_eq!(event.payload["deadline_reached"], false);
    let _ = &team.child_run;
}

#[tokio::test]
async fn a_child_stopped_by_its_deadline_says_so() {
    let team = team("child-finished-deadline", true, false).await;
    let store = &team.runtime.owned.durable_state_store;
    let deadline_ms = crate::session::unix_epoch_ms() + 60_000;
    store
        .agent_lifecycle(Operation::SetDeadline {
            room: team.room.clone(),
            agent: team.child.clone(),
            deadline: deadline_ms,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::ReachDeadline {
            room: team.room.clone(),
            agent: team.child.clone(),
            now: deadline_ms + 1,
            stopped: true,
        })
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["deadline_reached"], true, "ORCH-06");
    assert_eq!(events[0].payload["success"], false);
    assert_eq!(events[0].payload["outcome"]["kind"], "cancelled", "ORCH-16");
}

#[tokio::test]
async fn a_finish_wakes_a_parked_parent_in_the_same_second() {
    let team = team("child-finished-wake", false, false).await;
    let started = std::time::Instant::now();
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    let event = &child_finished(&team)[0];
    assert!(
        event.attempted_at_ms.is_some(),
        "the parked parent was woken by the finish itself, not by a later sweep: {event:?}"
    );
    assert!(event.prompt_id.is_some(), "{event:?}");
}

#[tokio::test]
async fn a_finish_steers_a_running_parent() {
    let team = team("child-finished-steer", false, true).await;
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let event = &child_finished(&team)[0];
    assert!(
        event.target_prompt_id.is_some() && event.attempted_at_ms.is_some(),
        "the running parent's turn is the steer target: {event:?}"
    );
}

#[tokio::test]
async fn a_deadline_that_stops_a_working_child_tells_the_parent() {
    let team = team_with("child-finished-deadline-stop", None, false).await;
    let deadline_ms = crate::session::unix_epoch_ms() + 30;
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::SetDeadline {
            room: team.room.clone(),
            agent: team.child.clone(),
            deadline: deadline_ms,
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["deadline_reached"], true);
    assert_eq!(events[0].payload["outcome"]["kind"], "cancelled");
    assert!(events[0].urgent);
}

// A4: the parent's deadline ends the sub-agents below it.
#[tokio::test]
async fn a_parents_deadline_stops_the_working_sub_agent_below_it() {
    let team = team_with("child-finished-parent-deadline", None, false).await;
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::SetDeadline {
            room: team.room.clone(),
            agent: team.parent.clone(),
            deadline: crate::session::unix_epoch_ms() + 30,
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert_eq!(
        team.runtime
            .owned
            .provider_store
            .get_run(&team.child_run)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended,
        "the child's run ended with its parent's deadline"
    );
    assert!(
        team.runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&team.room), Some(&team.child))
            .unwrap()
            .iter()
            .all(|task| task.state
                == crate::durable_state::agent_lifecycle::ExecutionState::Cancelled)
    );
}
