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
    pub(super) _worktree: crate::test_support::TestWorktree,
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
    _worktree: crate::test_support::TestWorktree,
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
        _worktree,
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
        _worktree,
    }
}

/// A parent and `children` sub-agents it spawned, each with an active turn and a task.
pub(super) async fn family(
    label: &str,
    children: usize,
    settle_cancelled: Option<bool>,
    parent_running: bool,
) -> Family {
    family_of(label, children, settle_cancelled, parent_running, true).await
}

/// `with_spawn_key: false` makes the children agents a person moved under the parent: they have no spawn key.
pub(super) async fn family_of(
    label: &str,
    children: usize,
    settle_cancelled: Option<bool>,
    parent_running: bool,
    with_spawn_key: bool,
) -> Family {
    family_in(
        label,
        children,
        settle_cancelled,
        parent_running,
        with_spawn_key,
        None,
    )
    .await
}

/// `history_root` keeps the Room's history (and its public projection) on disk, so a test can
/// read a turn back through the history tool.
pub(super) async fn family_with_history_root(
    label: &str,
    children: usize,
    settle_cancelled: Option<bool>,
    parent_running: bool,
    history_root: Option<std::path::PathBuf>,
) -> Family {
    family_in(
        label,
        children,
        settle_cancelled,
        parent_running,
        true,
        history_root,
    )
    .await
}

async fn family_in(
    label: &str,
    children: usize,
    settle_cancelled: Option<bool>,
    parent_running: bool,
    with_spawn_key: bool,
    history_root: Option<std::path::PathBuf>,
) -> Family {
    let mut config = crate::config::DaemonConfig::for_tests();
    if let Some(root) = history_root {
        config = config.with_session_history_root(root);
    }
    family_from_config(
        label,
        children,
        settle_cancelled,
        parent_running,
        with_spawn_key,
        config,
    )
    .await
}

async fn family_from_config(
    label: &str,
    children: usize,
    settle_cancelled: Option<bool>,
    parent_running: bool,
    with_spawn_key: bool,
    mut config: crate::config::DaemonConfig,
) -> Family {
    let worktree = crate::test_support::TestWorktree::new(label);
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
        let mut request = crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
            .with_alias(if index == 0 {
                "child".to_owned()
            } else {
                format!("child{}", index + 1)
            })
            .with_parent_agent_id(parent.id());
        if with_spawn_key {
            request = request.with_spawn_key(format!("spawn-{label}-{index}"));
        }
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(request)
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
        _worktree: worktree,
    }
}

pub(super) fn child_finished(
    team: &Team,
) -> Vec<crate::durable_state::agent_lifecycle::InboxEvent> {
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
            deadline: Some(deadline_ms),
            inherited: Vec::new(),
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
async fn provider_failure_reason_reaches_the_parent() {
    let team = team_with("provider-failure-reason", None, true).await;
    let runtime = &team.runtime;
    let parent_run = runtime
        .owned
        .provider_store
        .get_run_for_agent(&team.room, &team.parent)
        .expect("the parent has an active run");
    let token = parent_run
        .runtime_mcp_auth_token()
        .expect("parent's tool auth");
    let get_child = || {
        runtime.dispatch_authenticated_runtime_tool_call(
            token,
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
            serde_json::json!({"agent": "@child"}),
        )
    };
    let reason = "The selected model refused this request.";
    runtime
        .fail_owned_provider_prompt(&team.room, &team.child_run, reason, true)
        .await
        .unwrap();

    let events = child_finished(&team);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["outcome"]["reason"], reason);
    assert_eq!(events[0].payload["outcome"]["kind"], "model_refused");
    assert!(events[0].payload["finished_at_ms"].as_u64().is_some());
    let child = get_child().await.unwrap();
    assert!(child.ok, "{:?}", child.payload);
    assert_eq!(child.payload["agent"]["state"], "error");
    assert_eq!(
        child.payload["agent"]["failure"]["outcome"]["reason"],
        reason
    );
    assert!(child.payload["agent"]["failure"]["finished_at_ms"]
        .as_u64()
        .is_some());
}

#[tokio::test]
async fn provider_failure_reason_deadline_overrides_the_last_error() {
    let team = team_with("provider-failure-deadline", None, true).await;
    let runtime = &team.runtime;
    runtime
        .fail_owned_provider_prompt(
            &team.room,
            &team.child_run,
            "Connection closed unexpectedly.",
            true,
        )
        .await
        .unwrap();
    let parent_run = runtime
        .owned
        .provider_store
        .get_run_for_agent(&team.room, &team.parent)
        .expect("the parent has an active run");
    let token = parent_run
        .runtime_mcp_auth_token()
        .expect("parent's tool auth");
    let get_child = || {
        runtime.dispatch_authenticated_runtime_tool_call(
            token,
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
            serde_json::json!({"agent": "@child"}),
        )
    };
    assert_eq!(
        get_child().await.unwrap().payload["agent"]["state"],
        "error"
    );
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::SetDeadline {
            room: team.room.clone(),
            agent: team.child.clone(),
            deadline: Some(now),
            inherited: Vec::new(),
        })
        .unwrap();
    runtime.tick_agent_deadlines(now).await.unwrap();
    let child = get_child().await.unwrap();
    assert_eq!(child.payload["agent"]["state"], "ended (deadline)");
    assert!(child.payload["agent"]["failure"].is_null());
    let listed = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    let child = listed.payload["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["alias"] == "child")
        .unwrap();
    assert_eq!(child["state"], "ended (deadline)");
    let session = runtime
        .session_agent_snapshot(&team.room, &team.parent)
        .unwrap()
        .0;
    let activity = runtime.agent_activity_for_session(&session);
    assert_eq!(
        serde_json::to_value(&activity[&team.child]).unwrap()["status"],
        "ended_deadline"
    );
}

#[test]
fn provider_failure_reason_survives_kernel_restart() {
    let root = crate::test_support::TestTempDir::new("provider-failure-restart");
    let mut config =
        crate::config::DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.room_agent_tools = true;
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.credential_vault.path = root.join("vault.json").display().to_string();
    let executor = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    };
    let reason = "Connection closed while waiting for the provider.";

    let (room, parent, worktree, ended_at) = executor().block_on(async {
        let Family {
            runtime,
            room,
            parent,
            parent_run,
            kids,
            _worktree,
        } = family_from_config("provider-failure-cold", 1, None, true, true, config.clone()).await;
        let kid = &kids[0];
        runtime
            .fail_owned_provider_prompt(&room, &kid.child_run, reason, true)
            .await
            .unwrap();
        let settlement = runtime
            .owned
            .operational_history_store
            .load_prompt_settlement_event(&room, &kid.child, kid.child_prompt.id())
            .unwrap()
            .unwrap();
        let ended_at =
            settlement.metadata[crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY].clone();
        runtime
            .owned
            .complete_local_prompt_without_advance(&room, &parent, Some(&parent_run))
            .unwrap()
            .expect("the parent ends its inspection turn before restarting");
        (room, parent, _worktree, ended_at)
    });

    // Stop the first runtime's background tasks as well as dropping its kernel state.
    executor().block_on(async {
        let mut app = DaemonApp::bootstrap(config).unwrap();
        crate::test_support::admit_room_test_turn(&mut app, &room, &parent);
        let parent_run = app.providers().get_run_for_agent(&room, &parent).unwrap();
        let token = parent_run.runtime_mcp_auth_token().unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let child = runtime
            .dispatch_authenticated_runtime_tool_call(
                token,
                crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
                serde_json::json!({"agent": "@child"}),
            )
            .await
            .unwrap();
        assert!(child.ok, "{:?}", child.payload);
        assert_eq!(child.payload["agent"]["state"], "error");
        assert_eq!(
            child.payload["agent"]["failure"],
            serde_json::json!({
                "outcome": { "kind": "network", "reason": reason, "retryable": true },
                "finished_at_ms": ended_at,
            })
        );
    });
    drop(worktree);
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

/// The wake carries the child's report, and the kernel closes the event the parent no longer has to
/// read: the parent finishes on the wake alone, with no inbox read or acknowledgement.
#[tokio::test]
async fn a_finish_wake_carries_the_childs_report_and_closes_its_event() {
    let team = team("child-finished-inline", false, true).await;
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let event = &child_finished(&team)[0];
    let prompt_id = event.prompt_id.clone().expect("the wake prompt");
    let entries = team
        .runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&team.room, Some(&team.parent))
        .unwrap();
    let wake = entries
        .iter()
        .find(|entry| {
            entry.merge_key.as_deref()
                == Some(crate::history::steering_prompt_merge_key(&prompt_id).as_str())
        })
        .unwrap_or_else(|| panic!("no steered wake row: {entries:?}"));
    for expected in [
        "a sub-agent of yours: a report",
        "\"success\":true",
        &team.task.task_id,
    ] {
        assert!(
            wake.text.contains(expected),
            "missing {expected}: {}",
            wake.text
        );
    }
    assert!(!wake.text.contains("chariox.events.inbox"), "{}", wake.text);
    assert_eq!(event.state, "handled", "{event:?}");
}

/// The steered wake reaches the provider as a user message of the running turn.
/// The kernel must own it in history: otherwise the transcript observer imports
/// that message as an external turn and stores the turn's answer a second time.
#[tokio::test]
async fn a_steering_finish_is_recorded_as_a_kernel_owned_prompt() {
    let team = team("child-finished-steer-history", false, true).await;
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let event = &child_finished(&team)[0];
    let prompt_id = event.prompt_id.clone().expect("the wake prompt");
    let entries = team
        .runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&team.room, Some(&team.parent))
        .unwrap();
    let steer = entries
        .iter()
        .find(|entry| {
            entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                && entry.merge_key.as_deref()
                    == Some(crate::history::steering_prompt_merge_key(&prompt_id).as_str())
        })
        .unwrap_or_else(|| panic!("the steered wake has no kernel-owned prompt row: {entries:?}"));
    assert!(steer.text.starts_with("Inbox: 1 new event"), "{steer:?}");
    assert_ne!(
        steer.prompt_origin,
        Some(crate::session::PromptOrigin::External),
        "{steer:?}"
    );
}

/// The wake is a kernel instruction to the model. The kernel keeps it as a
/// steering row of the parent's turn, but no client projection shows it as
/// something the person wrote; a person's own steer in the same turn stays.
#[tokio::test]
async fn a_steering_finish_is_not_shown_to_clients_as_a_user_prompt() {
    let team = team("child-finished-steer-hidden", false, true).await;
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let wake = child_finished(&team)[0]
        .prompt_id
        .clone()
        .expect("the wake prompt");
    let wake_row = team
        .runtime
        .owned
        .operational_history_store
        .load_session_events(&team.room, Some(&team.parent))
        .unwrap()
        .into_iter()
        .find(|event| {
            event.metadata.get("merge_key").and_then(|key| key.as_str())
                == Some(crate::history::steering_prompt_merge_key(&wake).as_str())
        })
        .expect("the wake's steering row");
    let turn_prompt = wake_row.prompt_id.clone().expect("the running turn");
    let attachment = wake_row.metadata["source_attachment_id"]
        .as_str()
        .expect("the wake's attachment")
        .to_owned();
    team.runtime
        .owned
        .append_steering_prompt_history(
            &team.room,
            wake_row
                .provider_run_id
                .as_deref()
                .expect("the parent's run"),
            &turn_prompt,
            &attachment,
            &team.parent,
            "person-steer-1",
            "please also check the logs",
            &[],
            crate::session::PromptOrigin::Chariox,
        )
        .unwrap();
    for include_prefetched_content in [false, true] {
        let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } =
            crate::runtime::history_requests::execute_session_history_outline_request(
                team.runtime.owned.operational_history_store.clone(),
                crate::local::GetSessionHistoryOutlineRequest {
                    session_id: team.room.clone(),
                    agent_ids: Some(vec![team.parent.clone()]),
                    latest_prompt_count: Some(4),
                    include_prefetched_content,
                    after_sequence: None,
                    cursor: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("expected the history outline");
        };
        let text = serde_json::to_string(&agents).unwrap();
        assert!(
            !text.contains("Inbox: at least") && !text.contains(&wake),
            "the kernel's wake shows as a user prompt (prefetch {include_prefetched_content}): {text}"
        );
        assert!(
            text.contains("please also check the logs"),
            "the person's steer is gone (prefetch {include_prefetched_content}): {text}"
        );
    }
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
            deadline: Some(deadline_ms),
            inherited: Vec::new(),
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
            deadline: Some(crate::session::unix_epoch_ms() + 30),
            inherited: Vec::new(),
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

// The owner revives only the sub-agent a parent's deadline ended. The parent's finished cleanup
// must not stop the work the revived sub-agent starts afterwards.
#[tokio::test]
async fn a_revived_sub_agents_new_work_survives_its_parents_ended_deadline() {
    let team = team_with("child-revived-after-parent-deadline", None, false).await;
    let runtime = &team.runtime;
    let store = &runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::SetDeadline {
            room: team.room.clone(),
            agent: team.parent.clone(),
            deadline: Some(crate::session::unix_epoch_ms() + 30),
            inherited: Vec::new(),
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    runtime
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    let parent_receipt = store.agent_deadline(&team.room, &team.parent).unwrap().unwrap();
    assert!(parent_receipt.stopped_at_ms.is_some(), "{parent_receipt:?}");
    assert_eq!(
        runtime.owned.provider_store.get_run(&team.child_run).unwrap().state(),
        crate::provider::ProviderRunState::Ended
    );

    // Only the child is revived: its limit is cleared, the parent's stays ended.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    runtime
        .set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: team.room.clone(),
                agent_id: team.child.clone(),
                deadline_ms: None,
            },
        )
        .await
        .unwrap();
    let attachment = runtime
        .owned
        .attachment_store
        .list_session_attachment_ids(&team.room)
        .into_iter()
        .next()
        .expect("the room has an attachment");
    runtime
        .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
            session_id: team.room.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "revived-prompt",
                &attachment,
                &team.child,
                "continue",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .await
        .unwrap();
    let session = runtime.owned.session_store.get_session(&team.room).unwrap();
    let work = |runtime: &KernelRuntimeState| {
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &team.child);
        active.is_some() || !queued.is_empty()
    };
    assert!(work(runtime), "the revived child has new work");

    // The scheduler's next tick sees the parent's ended deadline and the child's new work.
    runtime
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert!(work(runtime), "the parent's old deadline stopped the revived child's new work");
    let live = runtime
        .owned
        .provider_store
        .get_run_for_agent(&team.room, &team.child)
        .expect("the revived child's turn has a run");
    assert_ne!(live.state(), crate::provider::ProviderRunState::Ended);
}

/// The parent's `child_finished`, read until `done` holds.
async fn child_finished_when(
    runtime: &KernelRuntimeState,
    room: &str,
    parent: &str,
    done: impl Fn(&crate::durable_state::agent_lifecycle::InboxEvent) -> bool,
) -> crate::durable_state::agent_lifecycle::InboxEvent {
    let read = || {
        runtime
            .owned
            .durable_state_store
            .agent_inbox(room, parent, 0)
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "child_finished")
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match read() {
            Some(event) if done(&event) => return event,
            last if std::time::Instant::now() >= until => {
                panic!("child_finished never reached the expected state: {last:?}")
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
}

#[tokio::test]
async fn a_finish_while_the_parents_turn_settles_wakes_the_parent_when_it_ends() {
    let family = family("child-finished-settling", 1, Some(false), true).await;
    let kid = &family.kids[0];
    // The parent's turn is settling: its output pump holds the run's lane.
    let lane = family
        .runtime
        .provider_runtime_lanes
        .acquire(&family.parent_run)
        .await;
    let settled = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        family.runtime.finish_agent_task_settlement(
            Some((kid.task.clone(), kid.correction)),
            &kid.child_prompt,
        ),
    )
    .await;
    assert!(
        settled.is_ok(),
        "the child's settlement does not wait for the parent's settling turn"
    );
    settled.unwrap().unwrap();
    // The turn ends; its pump lets the lane go a moment later.
    family
        .runtime
        .owned
        .complete_local_prompt_without_advance(
            &family.room,
            &family.parent,
            Some(&family.parent_run),
        )
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    drop(lane);
    let event = child_finished_when(&family.runtime, &family.room, &family.parent, |event| {
        event.prompt_id.is_some() && event.target_prompt_id.is_none()
    })
    .await;
    assert!(
        !matches!(event.state.as_str(), "pending" | "blocked"),
        "the parked parent is woken with its child's finish: {event:?}"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: the child's settling turn holds the child's run lane,
// and a workflow cancel holding the parked parent's lane waits for the child's lane in
// turn. Waking the parent must not wait for the parent's lane inside the child's settlement.
#[tokio::test]
async fn a_finish_wakes_a_parked_parent_without_waiting_for_its_lane() {
    let family = family("child-finished-parked-lane", 1, Some(false), false).await;
    let kid = &family.kids[0];
    let lane = family
        .runtime
        .provider_runtime_lanes
        .acquire(&family.parent_run)
        .await;
    let settled = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        family.runtime.finish_agent_task_settlement(
            Some((kid.task.clone(), kid.correction)),
            &kid.child_prompt,
        ),
    )
    .await;
    assert!(
        settled.is_ok(),
        "the child's settlement does not wait for its parked parent's run lane"
    );
    settled.unwrap().unwrap();
    drop(lane);
    let event = child_finished_when(&family.runtime, &family.room, &family.parent, |event| {
        event.prompt_id.is_some() && !matches!(event.state.as_str(), "pending" | "blocked")
    })
    .await;
    assert!(event.target_prompt_id.is_none(), "{event:?}");
}

#[tokio::test]
async fn a_cancelled_child_tells_the_parent_once_whichever_route_ends_it() {
    let team = team_with("child-finished-cancelled-once", None, false).await;
    let Outcome::Task(cancelled) = team
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::CancelTask {
            task: team.task.task_id.clone(),
            owner: "local".into(),
            revision: team.task.revision,
        })
        .unwrap()
    else {
        panic!("cancelled")
    };
    // The cancellation cascade stops the turn; its end then settles the task again.
    team.runtime
        .cancel_agent_task_resources(&cancelled)
        .await
        .unwrap();
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.child, Some(&team.child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((cancelled, false)), &team.child_prompt)
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["success"], false);
    assert_eq!(events[0].payload["outcome"]["kind"], "cancelled");
    assert!(events[0].urgent);
}

/// Case H: the owner's cancel of a child's card reached the parent as `child_finished`
/// "cancelled" with an empty reason; every cancel outcome says why.
#[tokio::test]
async fn a_cancelled_childs_finish_says_why() {
    let team = team_with("child-finished-cancel-reason", None, false).await;
    let Outcome::Task(cancelled) = team
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::CancelTask {
            task: team.task.task_id.clone(),
            owner: "local".into(),
            revision: team.task.revision,
        })
        .unwrap()
    else {
        panic!("cancelled")
    };
    assert!(!cancelled.reason.is_empty());
    team.runtime
        .cancel_agent_task_resources(&cancelled)
        .await
        .unwrap();
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.child, Some(&team.child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((cancelled.clone(), false)), &team.child_prompt)
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["outcome"]["kind"], "cancelled");
    assert_eq!(
        events[0].payload["outcome"]["reason"],
        cancelled.reason.as_str(),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_child_turn_cancelled_by_its_provider_says_why() {
    let team = team("child-finished-turn-cancel-reason", true, false).await;
    assert_eq!(
        team.task.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
    );
    team.runtime
        .finish_agent_task_settlement(
            Some((team.task.clone(), team.correction)),
            &team.child_prompt,
        )
        .await
        .unwrap();
    let events = child_finished(&team);
    assert_eq!(events.len(), 1, "{events:?}");
    let reason = events[0].payload["outcome"]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(!reason.is_empty(), "{events:?}");
}

async fn end_kid(family: &Family, kid: &Kid) {
    let store = &family.runtime.owned.durable_state_store;
    let Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: family.room.clone(),
            agent: kid.child.clone(),
            prompt: kid.child_prompt.id().into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    family
        .runtime
        .owned
        .complete_local_prompt_without_advance(&family.room, &kid.child, Some(&kid.child_run))
        .unwrap();
    family
        .runtime
        .finish_agent_task_settlement(Some((task, correction)), &kid.child_prompt)
        .await
        .unwrap();
}

fn wake_prompts(family: &Family) -> Vec<Option<String>> {
    family
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&family.room, &family.parent, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "child_finished")
        .map(|event| event.prompt_id)
        .collect()
}

/// AO-IN-02: workers that finish together reach their parent in one wake. A finish that leaves
/// a sibling working waits a moment for the others instead of waking the parent three times.
#[tokio::test]
async fn children_that_finish_together_wake_their_parent_once() {
    let family = family("child-finished-together", 3, None, false).await;
    for kid in &family.kids {
        end_kid(&family, kid).await;
    }
    let prompts = wake_prompts(&family);
    assert_eq!(prompts.len(), 3, "{prompts:?}");
    assert!(
        prompts[0].is_some() && prompts.iter().all(|prompt| *prompt == prompts[0]),
        "one wake carries the three finishes: {prompts:?}"
    );
}

/// A sibling that keeps working does not hold the parent for ever: the window ends.
#[tokio::test]
async fn a_finish_waits_for_its_siblings_only_for_a_moment() {
    let family = family("child-finished-window", 2, None, false).await;
    end_kid(&family, &family.kids[0]).await;
    assert_eq!(wake_prompts(&family), [None], "held for the sibling");
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    let prompts = wake_prompts(&family);
    assert!(
        prompts[0].is_some(),
        "the window ended and woke the parent: {prompts:?}"
    );
}
