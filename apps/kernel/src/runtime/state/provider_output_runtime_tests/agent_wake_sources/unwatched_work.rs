//! Row 554: an idle agent with nothing armed is stranded silently. The kernel helps and never
//! blocks: it adopts the agent's still-running background commands as process wakes, arms a
//! backstop for a stated wait, never refuses a turn end, and starts the turn of an idle agent a
//! queued message is waiting for.
//!
//! RED on main: cells 1, 2 and 4. Cell 3 (no turn end is refused) holds on main and must keep
//! holding. Each assertion names the cell it belongs to.
use super::super::child_finished::{team_with, Team};
use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::terminal::TerminalOutputKind;
use std::os::unix::process::CommandExt as _;

fn text_chunk(text: &str) -> ProviderPromptChunk {
    ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderOutput,
        merge_key: None,
        bytes: text.as_bytes().to_vec(),
    }
}

async fn say(team: &Team, text: &str) {
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            ProviderPromptSignalBatch {
                chunks: vec![text_chunk(text)],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    team.runtime
        .flush_provider_history(&team.child_run)
        .await
        .unwrap();
}

/// A command the agent's shell left running in the background: detached from the shell's
/// process group, carrying the run's marker like every process the provider starts. Ended by
/// its exact PID when the test ends.
struct Background(std::process::Child);

impl Background {
    fn start(run: &str) -> Self {
        Self(
            std::process::Command::new("sleep")
                .arg("120")
                .env("CHARIOX_PROVIDER_RUN_ID", run)
                .process_group(0)
                .spawn()
                .expect("sleep starts"),
        )
    }
    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The turn ends the way the output pump ends it; the settlement is the ledger's answer.
async fn end_turn(team: &Team) -> Result<Option<(AgentTaskExecution, bool)>, DaemonError> {
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.child, Some(&team.child_run))
        .unwrap();
    team.runtime.owned.settle_agent_task(
        &team.room,
        &team.child,
        &team.child_prompt,
        &team.child_run,
        false,
    )
}

fn wakes(team: &Team) -> Vec<AgentWake> {
    team.runtime
        .owned
        .durable_state_store
        .agent_wakes(Some(&team.room), Some(&team.child))
        .unwrap()
}

fn notices(team: &Team) -> String {
    team.runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&team.room, Some(&team.child))
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == crate::history::SessionHistoryEntryKind::Notice)
        .map(|e| e.text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cell 1: an agent starts a background command and ends its turn without a watch. The kernel
/// adopts the command as a process wake and says which.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cell_1_a_background_command_left_running_at_turn_end_is_adopted_as_a_process_wake() {
    let team = team_with("nowake-adopt", None, false).await;
    let background = Background::start(&team.child_run);
    say(&team, "Started the six-run script in the background.").await;

    end_turn(&team).await.expect("the turn end is accepted");

    let adopted: Vec<_> = wakes(&team)
        .into_iter()
        .filter(|w| w.kind == "adopted_process" && w.pid == Some(background.pid()))
        .collect();
    assert_eq!(
        adopted.len(),
        1,
        "cell 1: the running command is a process wake of the agent: {:?}",
        wakes(&team)
    );
    let told = notices(&team);
    assert!(
        told.contains("sleep 120") && told.contains("woken"),
        "cell 1: the agent is told which command was adopted: {told}"
    );

    // The command ends: the wake ends with it and the agent has an event to wake on.
    let wake_id = adopted[0].id.clone();
    drop(background);
    let store = &team.runtime.owned.durable_state_store;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline
        && store
            .agent_wake(&wake_id)
            .unwrap()
            .is_some_and(|w| matches!(w.state.as_str(), "starting" | "running"))
    {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let ended = store.agent_wake(&wake_id).unwrap().unwrap();
    assert!(
        !matches!(ended.state.as_str(), "starting" | "running"),
        "cell 1: the command's end ended its wake: {ended:?}"
    );
    assert!(
        store
            .agent_inbox(&team.room, &team.child, 0)
            .unwrap()
            .iter()
            .any(|event| event.source_id == wake_id),
        "cell 1: the end of the command is an event for the agent"
    );
}

/// Cell 2: an agent ends its turn saying it waits for the review, with nothing armed. The kernel
/// arms a backstop timer, tells the agent its kind and time, and the timer is the wake.
#[tokio::test]
async fn cell_2_a_stated_wait_with_nothing_armed_gets_a_backstop_timer() {
    let team = team_with("nowake-backstop", None, false).await;
    say(&team, "Pushed the PR. Waiting for the review.").await;
    let before = crate::session::unix_epoch_ms();

    end_turn(&team).await.expect("the turn end is accepted");

    let timers: Vec<_> = wakes(&team)
        .into_iter()
        .filter(|w| w.kind == "timer" && w.next_due_ms.is_some())
        .collect();
    assert_eq!(
        timers.len(),
        1,
        "cell 2: one backstop timer is armed: {:?}",
        wakes(&team)
    );
    let due = timers[0].next_due_ms.unwrap();
    let minute = 60_000;
    assert!(
        due > before + minute && due <= before + 31 * minute,
        "cell 2: the backstop is due within the 30 minutes a yield defaults to: {due} vs {before}"
    );
    let told = notices(&team);
    assert!(
        told.contains("backstop") && told.contains("timer"),
        "cell 2: the agent is told the kind and time of the wake: {told}"
    );
}

/// Cell 3: no turn end is refused, with unwatched work running or a wait stated. Holds on main.
#[tokio::test]
async fn cell_3_no_turn_end_is_refused() {
    let team = team_with("nowake-never-refused", None, false).await;
    let _background = Background::start(&team.child_run);
    say(&team, "Waiting for the build.").await;

    let settlement = end_turn(&team).await;

    assert!(settlement.is_ok(), "cell 3: {settlement:?}");
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert!(
        team.runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &team.child)
            .is_none(),
        "cell 3: the turn ended"
    );
}

/// Cell 4: a non-urgent message to an idle agent with nothing armed starts its turn. A working
/// agent keeps queueing it for its next turn (`a_turn_started_by_the_person_is_told_of_the_messages_queued_for_it`
/// covers the queue read).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cell_4_a_queued_message_to_an_idle_agent_with_nothing_armed_starts_its_turn() {
    // The parent works; its child answered and is idle with no wake.
    let team = team_with("nowake-idle-message", Some(false), true).await;
    assert!(wakes(&team).is_empty(), "the child has nothing armed");
    let parent_run = team
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&team.room, &team.parent)
        .expect("the parent has an active run");
    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let origin = team
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.parent)
        .expect("the parent has a running turn")
        .id()
        .to_string();

    let sent = Box::pin(team.runtime.dispatch_authenticated_runtime_tool_call(
        parent_run.runtime_mcp_auth_token().unwrap(),
        crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
        serde_json::json!({"agent": "@child", "message": "go ahead", "origin_prompt_id": origin}),
    ))
    .await
    .unwrap();
    assert!(sent.ok, "{:?}", sent.payload);
    // What delivery does for a message that can start its recipient's turn.
    Box::pin(team.runtime.deliver_agent_inbox(&team.room, &team.child))
        .await
        .unwrap();

    let message = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.child, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "message")
        .expect("the child's inbox holds the message");
    assert!(
        message.prompt_id.is_some(),
        "cell 4: the idle child's turn started for the message ({:?}, reply {:?})",
        message.state,
        sent.payload
    );
}
