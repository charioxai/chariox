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
struct Background {
    pid: u32,
    /// Set while the command is still this process's child; a detached one is the init's.
    child: Option<std::process::Child>,
}

impl Background {
    /// A command the agent's shell left behind: detached from the shell, which is gone, so it is
    /// nobody's child but init's (a `nohup cmd &`).
    fn start(run: &str) -> Self {
        Self::start_owned_by(run, crate::provider::process_owner::current_marker())
    }
    /// A command of run `run` whose kernel is `owner`: provider run ids repeat from kernel to kernel.
    fn start_owned_by(run: &str, owner: Option<String>) -> Self {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg("nohup sleep 120 >/dev/null 2>&1 & echo $!")
            .env("CHARIOX_PROVIDER_RUN_ID", run)
            .envs(owner.map(|owner| (crate::provider::process_owner::OWNER_ENV, owner)))
            .output()
            .expect("sh starts");
        let pid = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("the shell prints the command's pid");
        Self { pid, child: None }
    }
    /// A process still below the kernel: what a provider starts and keeps (its own process, its
    /// confinement wrapper, its helpers).
    fn attached(run: &str) -> Self {
        let child = std::process::Command::new("sleep")
            .arg("120")
            .env("CHARIOX_PROVIDER_RUN_ID", run)
            .envs(
                crate::provider::process_owner::current_marker()
                    .map(|owner| (crate::provider::process_owner::OWNER_ENV, owner)),
            )
            .process_group(0)
            .spawn()
            .expect("sleep starts");
        Self {
            pid: child.id(),
            child: Some(child),
        }
    }
    fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for Background {
    /// Ends exactly this process, never a group or a name.
    fn drop(&mut self) {
        match self.child.as_mut() {
            Some(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            None => {
                // SAFETY: one signal to a pid this test started and still owns the identity of.
                unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) };
            }
        }
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

    // The command may never end: an idle agent's message must not wait for it.
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(
            crate::durable_state::agent_lifecycle::occurrence(
                &team.room,
                &team.child,
                "peer",
                "while-it-runs",
                "message",
                serde_json::json!({"message": "go ahead"}),
            ),
        ))
        .unwrap();
    assert!(
        team.runtime
            .owned
            .release_queued_for_stranded(&team.room, &team.child)
            .unwrap(),
        "cell 1: an adopted command held the idle agent's message: {:?}",
        team.runtime.owned.what_arms_agent(&team.room, &team.child)
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

/// Live finding (drill): provider run ids repeat from kernel to kernel, so a process of another
/// kernel's run with the same id must never be adopted. Only the kernel's own marker counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cell_1_a_command_of_another_kernels_run_with_the_same_id_is_not_adopted() {
    let team = team_with("nowake-foreign-run", None, false).await;
    let ours = Background::start(&team.child_run);
    let foreign =
        Background::start_owned_by(&team.child_run, Some("4242:1:other-boot:ns:home".into()));
    say(&team, "Started it in the background.").await;

    end_turn(&team).await.expect("the turn end is accepted");

    let adopted: Vec<_> = wakes(&team)
        .into_iter()
        .filter(|w| w.kind == "adopted_process")
        .filter_map(|w| w.pid)
        .collect();
    assert!(
        adopted.contains(&ours.pid()),
        "cell 1: ours is adopted: {adopted:?}"
    );
    assert!(
        !adopted.contains(&foreign.pid()),
        "cell 1: another kernel's command was adopted: {adopted:?}"
    );
}

/// Live finding (drill): the provider's own process and its confinement wrapper carry the run's
/// marker and are still below the kernel. They are the provider's, never an agent's leftovers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cell_1_a_process_still_below_the_kernel_is_the_providers_and_is_not_adopted() {
    let team = team_with("nowake-provider-own", None, false).await;
    let provider_own = Background::attached(&team.child_run);
    let left_behind = Background::start(&team.child_run);
    say(&team, "Started it in the background.").await;

    end_turn(&team).await.expect("the turn end is accepted");

    let adopted: Vec<_> = wakes(&team)
        .into_iter()
        .filter(|w| w.kind == "adopted_process")
        .filter_map(|w| w.pid)
        .collect();
    assert!(
        adopted.contains(&left_behind.pid()),
        "cell 1: the command left behind is adopted: {adopted:?}"
    );
    assert!(
        !adopted.contains(&provider_own.pid()),
        "cell 1: the provider's own process was adopted: {adopted:?}"
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

/// Cell 2, review P2: the backstop covers an idle interval. When another turn resumes the agent
/// and finishes before the deadline, the wait was satisfied: the timer's firing wakes nobody.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cell_2_a_backstop_does_not_wake_an_agent_another_turn_already_resumed() {
    let team = team_with("nowake-backstop-superseded", None, false).await;
    say(&team, "Pushed the PR. Waiting for the review.").await;
    end_turn(&team).await.expect("the turn end is accepted");
    let store = &team.runtime.owned.durable_state_store;
    let backstop = wakes(&team)
        .into_iter()
        .find(|w| w.kind == "timer")
        .expect("the backstop is armed");

    // The review arrives as a person's prompt five minutes later; the agent finishes it.
    let later = "later-turn";
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: team.room.clone(),
            agent: team.child.clone(),
            prompt: later.into(),
            run: Some(team.child_run.clone()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.child.clone(),
            prompt: later.into(),
            run: team.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    // Review P2: the idle agent is not "armed" by a backstop a later turn already superseded: a
    // message sent now starts its turn, it does not wait for the old deadline.
    let message = crate::durable_state::agent_lifecycle::occurrence(
        &team.room,
        &team.child,
        "peer",
        "after-the-review",
        "message",
        serde_json::json!({"message": "go ahead"}),
    );
    store.agent_lifecycle(Operation::Occur(message)).unwrap();
    assert!(
        team.runtime
            .owned
            .release_queued_for_stranded(&team.room, &team.child)
            .unwrap(),
        "cell 2: a superseded backstop still holds the idle agent's message: {:?}",
        team.runtime.owned.what_arms_agent(&team.room, &team.child)
    );

    // The deadline passes.
    store
        .agent_lifecycle(Operation::FireWakes {
            now: backstop.next_due_ms.unwrap() + 1_000,
        })
        .unwrap();
    Box::pin(team.runtime.deliver_agent_inbox(&team.room, &team.child))
        .await
        .unwrap();

    let woken = store
        .agent_inbox(&team.room, &team.child, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.source_id == backstop.id && event.prompt_id.is_some())
        .count();
    assert_eq!(
        woken,
        0,
        "cell 2: the satisfied wait woke the agent again; tasks {:?}; events {:?}; backstop task {}",
        store
            .agent_tasks(Some(&team.room), Some(&team.child))
            .unwrap()
            .iter()
            .map(|t| (t.task_id.clone(), t.prompt_id.clone(), t.state.clone()))
            .collect::<Vec<_>>(),
        store
            .agent_inbox(&team.room, &team.child, 0)
            .unwrap()
            .iter()
            .map(|e| (e.kind.clone(), e.state.clone(), e.prompt_id.clone(), e.payload.clone()))
            .collect::<Vec<_>>(),
        backstop.task_id
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
    // The send itself starts the delivery: wait for the turn it starts.
    let find = || {
        team.runtime
            .owned
            .durable_state_store
            .agent_inbox(&team.room, &team.child, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "message")
            .expect("the child's inbox holds the message")
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while find().prompt_id.is_none() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let message = find();
    assert!(
        message.prompt_id.is_some(),
        "cell 4: the idle child's turn started for the message ({:?}, reply {:?})",
        message.state,
        sent.payload
    );
}
