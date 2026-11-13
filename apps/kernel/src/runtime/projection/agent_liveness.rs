//! What an agent waits on and how long it has been quiet, in the activity
//! other agents and clients already read. A state with no visible reason or
//! start is a limbo: every wait names its source and since when.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{AgentRuntimeActivity, AgentRuntimeStatus};
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, AgentWake, ExecutionState, Registration,
};
use crate::runtime::process_resources::{
    SessionProcess, SessionProcessOwner, SessionProcessState, WaitReason, Waiting,
};

/// A working agent with no transcript activity for this long is flagged.
pub(crate) const STALLED_AFTER_MS: u64 = 10 * 60 * 1000;
/// Activity times are floored to this: a streaming agent would otherwise
/// change its activity (and republish it) on every token.
const ACTIVITY_RESOLUTION_MS: u64 = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLiveness {
    /// The last transcript activity of the agent's provider run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<AgentWaitingOn>,
    /// Working, with no transcript activity for [`STALLED_AFTER_MS`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stalled: bool,
}

impl AgentLiveness {
    pub(crate) fn is_quiet(&self) -> bool {
        *self == Self::default()
    }

    /// A working agent waits on its provider until something says otherwise.
    pub(crate) fn of_provider(
        status: &AgentRuntimeStatus,
        last_activity_at_ms: Option<u64>,
        turn_started_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Self {
        if *status != AgentRuntimeStatus::Working {
            return Self::default();
        }
        // Quiet since the later of its last output and the start of this turn.
        let quiet_since = last_activity_at_ms.max(turn_started_at_ms);
        let floored = quiet_since.map(|at| at - at % ACTIVITY_RESOLUTION_MS);
        Self {
            last_activity_at_ms: floored,
            waiting_on: Some(AgentWaitingOn {
                kind: AgentWaitKind::Provider,
                detail: String::new(),
                resource: None,
                since_ms: floored.unwrap_or(now_ms),
            }),
            stalled: quiet_since.is_some_and(|at| now_ms.saturating_sub(at) > STALLED_AFTER_MS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWaitingOn {
    pub kind: AgentWaitKind,
    /// The command, timer or source it waits on; empty for the provider.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// What a command waits for room in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<WaitReason>,
    pub since_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWaitKind {
    Provider,
    /// A background command the agent yielded on.
    BackgroundCommand,
    Timer,
    /// Another source the agent yielded on.
    Source,
    /// Its command was stopped while growing into a full pool or disk.
    CommandPaused,
    /// Its command waits for room before it starts.
    CommandHeld,
}

/// A working agent in the health report: quiet since a time that is past the limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SilentAgentHealth {
    pub session_id: String,
    pub agent_id: String,
    pub quiet_since_ms: u64,
}

/// The agents whose turn is running and whose provider run wrote nothing for
/// longer than [`STALLED_AFTER_MS`]: what the health report flags. Quiet counts
/// from the later of the run's last output and the start of the admitted turn,
/// as the session liveness does: time in the queue is not quiet time. A prompt
/// with no admitted turn yet has not begun to be quiet.
pub(crate) fn silent_agents(
    agents: &[super::AgentRuntimeProjection],
    runs: &[crate::provider::RuntimeProviderRun],
    active_turns: &BTreeMap<String, crate::app::ActiveTurnState>,
    now_ms: u64,
) -> Vec<SilentAgentHealth> {
    agents
        .iter()
        .filter_map(|agent| {
            let prompt = agent.active_prompt.as_ref()?;
            if !matches!(
                prompt.status(),
                crate::session::PromptStatus::Running | crate::session::PromptStatus::Dispatching
            ) {
                return None;
            }
            let turn_started_at_ms =
                super::session_snapshot_projection::active_turn_for_session_agent(
                    active_turns,
                    &agent.session_id,
                    &agent.agent_id,
                )?
                .started_at_ms;
            let last_output = runs
                .iter()
                .filter(|run| run.agent_instance_id() == Some(agent.agent_id.as_str()))
                .map(crate::provider::RuntimeProviderRun::last_output_at_ms)
                .max();
            let quiet_since_ms = last_output.max(Some(turn_started_at_ms))?;
            (now_ms.saturating_sub(quiet_since_ms) > STALLED_AFTER_MS).then(|| SilentAgentHealth {
                session_id: agent.session_id.clone(),
                agent_id: agent.agent_id.clone(),
                quiet_since_ms,
            })
        })
        .collect()
}

/// Names what each agent waits on from the commands admission holds and the
/// wakes the agent yielded on. Command waits come first: they explain a
/// quiet provider.
pub(crate) fn annotate_waits(
    activity: &mut BTreeMap<String, AgentRuntimeActivity>,
    processes: &[SessionProcess],
    yielded: &YieldedSources,
) {
    for (agent_id, activity) in activity.iter_mut() {
        let waiting = command_wait(agent_id, processes).or_else(|| yielded_wait(agent_id, yielded));
        if let Some(waiting) = waiting {
            activity.liveness.waiting_on = Some(waiting);
        }
    }
}

fn command_wait(agent_id: &str, processes: &[SessionProcess]) -> Option<AgentWaitingOn> {
    processes
        .iter()
        .filter(|row| matches!(&row.owner, SessionProcessOwner::Agent(id) if id == agent_id))
        .filter_map(|row| match &row.state {
            SessionProcessState::Paused(waiting) => Some((AgentWaitKind::CommandPaused, waiting)),
            SessionProcessState::Held(waiting) => Some((AgentWaitKind::CommandHeld, waiting)),
            SessionProcessState::Running => None,
        })
        .min_by_key(|(_, waiting)| waiting.since_ms)
        .map(|(kind, waiting)| waiting_on(kind, waiting))
}

fn waiting_on(kind: AgentWaitKind, waiting: &Waiting) -> AgentWaitingOn {
    AgentWaitingOn {
        kind,
        detail: waiting.command_line.clone(),
        resource: Some(waiting.reason),
        since_ms: waiting.since_ms,
    }
}

/// The oldest thing a waiting task of the agent yielded on: a live wake
/// (background command or timer) or another source, such as a delegate, a
/// task or a workflow, that has not reported yet.
fn yielded_wait(agent_id: &str, yielded: &YieldedSources) -> Option<AgentWaitingOn> {
    let YieldedSources {
        tasks,
        wakes,
        registrations,
    } = yielded;
    let waiting_tasks = tasks
        .iter()
        .filter(|task| task.agent_id == agent_id && task.state == ExecutionState::Waiting)
        .collect::<Vec<_>>();
    let live_wakes = wakes
        .iter()
        .filter(|wake| {
            wake.agent_id == agent_id
                && waiting_tasks
                    .iter()
                    .any(|task| task.task_id == wake.task_id)
        })
        .filter(|wake| matches!(wake.state.as_str(), "scheduled" | "starting" | "running"))
        .collect::<Vec<_>>();
    let wake_waits = live_wakes.iter().map(|wake| AgentWaitingOn {
        kind: match wake.kind.as_str() {
            "process" | "terminal" => AgentWaitKind::BackgroundCommand,
            "timer" => AgentWaitKind::Timer,
            _ => AgentWaitKind::Source,
        },
        detail: if wake.command.is_empty() {
            wake.label.clone()
        } else {
            wake.command.join(" ")
        },
        resource: None,
        since_ms: wake.created_at_ms,
    });
    let source_waits = waiting_tasks.iter().flat_map(|task| {
        let wait = task.wait.as_ref();
        let live_wakes = &live_wakes;
        registrations
            .iter()
            .filter(move |registration| {
                registration.live
                    && registration.task_id == task.task_id
                    && wait.is_some_and(|wait| wait.registration_ids.contains(&registration.id))
                    && !live_wakes
                        .iter()
                        .any(|wake| wake.registration_id == registration.id)
            })
            .map(move |registration| AgentWaitingOn {
                kind: AgentWaitKind::Source,
                detail: registration.source_id.clone(),
                resource: None,
                since_ms: wait.map_or(task.last_progress_at_ms, |wait| wait.started_at_ms),
            })
    });
    wake_waits
        .chain(source_waits)
        .min_by_key(|wait| wait.since_ms)
}

/// What the waiting tasks of a room yielded on, read once for the activity of its agents.
#[derive(Default)]
pub(crate) struct YieldedSources {
    pub tasks: Vec<AgentTaskExecution>,
    pub wakes: Vec<AgentWake>,
    pub registrations: Vec<Registration>,
}

impl YieldedSources {
    pub(crate) fn of_room(
        store: &crate::durable_state::DurableKernelStateStore,
        room: &str,
    ) -> Self {
        let tasks = store.agent_tasks(Some(room), None).unwrap_or_default();
        let wakes = store.agent_wakes(Some(room), None).unwrap_or_default();
        let registrations = tasks
            .iter()
            .filter(|task| task.state == ExecutionState::Waiting)
            .flat_map(|task| store.agent_registrations(&task.task_id).unwrap_or_default())
            .collect();
        Self {
            tasks,
            wakes,
            registrations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::process_resources::ProcessSample;

    const MINUTE: u64 = 60 * 1000;

    fn idle_activity() -> AgentRuntimeActivity {
        serde_json::from_value(serde_json::json!({
            "status":"working","prompt_status":"running","busy":true,
            "active_prompt_count":1,"queued_prompt_count":0
        }))
        .unwrap()
    }

    fn row(agent: &str, state: SessionProcessState) -> SessionProcess {
        SessionProcess {
            remote_available: false,
            remote_refusal: None,
            estimate: None,
            placement: None,
            owner: SessionProcessOwner::Agent(agent.into()),
            process: ProcessSample {
                process_id: "p".into(),
                pid: 7,
                command: "cargo".into(),
                resident_set_bytes: 0,
                cpu_millis: 0,
                disk_io_bytes: None,
                disk_written_bytes: None,
                deleted_open_bytes: 0,
                outlived: false,
                sampled_at_ms: 0,
                age_ms: 0,
            },
            state,
        }
    }

    fn paused_since(since_ms: u64) -> SessionProcessState {
        SessionProcessState::Paused(Waiting {
            reason: WaitReason::Memory,
            position: 1,
            need_bytes: 0,
            free_bytes: 0,
            command_line: "cargo build".into(),
            cwd: None,
            since_ms,
        })
    }

    #[test]
    fn a_working_agent_waits_on_its_provider_since_its_last_activity() {
        let now = 100 * MINUTE;
        let liveness = AgentLiveness::of_provider(
            &AgentRuntimeStatus::Working,
            Some(now - 3 * MINUTE),
            None,
            now,
        );
        let waiting = liveness.waiting_on.unwrap();
        assert_eq!(waiting.kind, AgentWaitKind::Provider);
        assert_eq!(waiting.since_ms, now - 3 * MINUTE);
        assert!(!liveness.stalled);
    }

    #[test]
    fn a_working_agent_quiet_for_over_the_limit_is_flagged_and_a_new_turn_resets_it() {
        let now = 100 * MINUTE;
        let quiet = AgentLiveness::of_provider(
            &AgentRuntimeStatus::Working,
            Some(now - 11 * MINUTE),
            None,
            now,
        );
        assert!(quiet.stalled);
        // The turn started after the last output: quiet only since the turn began.
        let fresh = AgentLiveness::of_provider(
            &AgentRuntimeStatus::Working,
            Some(now - 11 * MINUTE),
            Some(now - MINUTE),
            now,
        );
        assert!(!fresh.stalled);
        assert!(
            AgentLiveness::of_provider(&AgentRuntimeStatus::Idle, Some(0), None, now)
                == AgentLiveness::default()
        );
    }

    #[test]
    fn a_running_turn_without_provider_output_for_over_the_limit_is_listed_as_silent() {
        let now = 100 * MINUTE;
        let prompt = || {
            crate::session::PromptQueueItem::new(
                "p",
                "attachment",
                "agent-1",
                "work",
                crate::session::PromptStatus::Running,
            )
        };
        let agent = |id: &str| super::super::AgentRuntimeProjection {
            session_id: "room".into(),
            agent_id: id.into(),
            active_prompt: Some(prompt()),
            next_queued_prompt: None,
            queued_prompt_count: 0,
        };
        let turn = |id: &str, started: u64| {
            let mut turn = crate::app::ActiveTurnState::new(
                "room".into(),
                id.into(),
                "p".into(),
                format!("run-{id}"),
            );
            turn.started_at_ms = started;
            (turn.provider_run_id.clone(), turn)
        };
        let turns = BTreeMap::from([
            turn("agent-1", now - 20 * MINUTE),
            turn("agent-2", now - MINUTE),
        ]);
        let silent = silent_agents(
            &[agent("agent-1"), agent("agent-2"), agent("agent-3")],
            &[],
            &turns,
            now,
        );
        // agent-3 has no admitted turn: nothing has started to be quiet.
        assert_eq!(silent.len(), 1);
        assert_eq!(silent[0].agent_id, "agent-1");
        assert_eq!(silent[0].quiet_since_ms, now - 20 * MINUTE);
    }

    #[test]
    fn a_paused_command_is_what_its_agent_waits_on_with_its_start() {
        let mut activity = BTreeMap::from([("a".to_string(), idle_activity())]);
        let processes = [
            row("a", SessionProcessState::Running),
            row("a", paused_since(5_000)),
            row("b", paused_since(1)),
        ];
        annotate_waits(&mut activity, &processes, &YieldedSources::default());
        let waiting = activity["a"].liveness.waiting_on.clone().unwrap();
        assert_eq!(waiting.kind, AgentWaitKind::CommandPaused);
        assert_eq!(waiting.resource, Some(WaitReason::Memory));
        assert_eq!(waiting.detail, "cargo build");
        assert_eq!(waiting.since_ms, 5_000);
    }

    fn task(agent: &str, id: &str, state: &str) -> AgentTaskExecution {
        serde_json::from_value(serde_json::json!({
            "task_id": id, "room_id": "room", "agent_id": agent, "prompt_id": "p",
            "provider_run_id": null, "revision": 1, "state": state, "reason": "",
            "obligations": [], "wait": null, "last_progress_at_ms": 0,
            "correction_used": false,
            "pending_prompt_id": null
        }))
        .unwrap()
    }

    fn wake(agent: &str, task: &str, kind: &str, state: &str, created: u64) -> AgentWake {
        serde_json::from_value(serde_json::json!({
            "id": format!("wake-{created}"), "task_id": task, "room_id": "room",
            "agent_id": agent, "registration_id": "r", "kind": kind, "label": "build",
            "state": state, "created_at_ms": created, "verified_at_ms": null,
            "next_due_ms": null, "interval_ms": null, "command": ["cargo", "build"],
            "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
            "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
            "last_sequence": null, "last_delivery": null, "last_delivered_at_ms": null,
            "last_acknowledged_at_ms": null, "alerted_sequence": null
        }))
        .unwrap()
    }

    #[test]
    fn an_agent_that_yielded_on_a_background_command_waits_on_it_since_it_was_armed() {
        let mut activity = BTreeMap::from([("a".to_string(), idle_activity())]);
        let tasks = [task("a", "t1", "waiting"), task("a", "t2", "done")];
        let wakes = [
            wake("a", "t1", "process", "running", 13_000),
            wake("a", "t1", "process", "exited", 1_000),
            wake("a", "t2", "process", "running", 2_000),
        ];
        annotate_waits(
            &mut activity,
            &[],
            &YieldedSources {
                tasks: tasks.to_vec(),
                wakes: wakes.to_vec(),
                registrations: Vec::new(),
            },
        );
        let waiting = activity["a"].liveness.waiting_on.clone().unwrap();
        assert_eq!(waiting.kind, AgentWaitKind::BackgroundCommand);
        assert_eq!(waiting.detail, "cargo build");
        assert_eq!(waiting.since_ms, 13_000);
    }

    fn waiting_task(
        agent: &str,
        id: &str,
        registration_ids: &[&str],
        started: u64,
    ) -> AgentTaskExecution {
        let mut task = task(agent, id, "waiting");
        task.wait = Some(crate::durable_state::agent_lifecycle::AgentWait {
            registration_ids: registration_ids.iter().map(|id| id.to_string()).collect(),
            deadline_ms: Some(started + 60 * MINUTE),
            started_at_ms: started,
            inbox_cursor: 0,
            long_wait_notified: false,
            last_checked_at_ms: started,
        });
        task
    }

    fn registration(task: &str, id: &str, source: &str, live: bool) -> Registration {
        serde_json::from_value(serde_json::json!({
            "id": id, "task_id": task, "source_id": source, "obligation_id": null,
            "source_cursor": 0, "live": live, "peer": false
        }))
        .unwrap()
    }

    #[test]
    fn an_agent_that_yielded_on_a_delegate_waits_on_that_source_since_the_yield() {
        let mut activity = BTreeMap::from([("a".to_string(), idle_activity())]);
        let tasks = [waiting_task("a", "t1", &["reg-1", "reg-2"], 7_000)];
        let registrations = [
            registration("t1", "reg-1", "delegate-task-9", true),
            registration("t1", "reg-2", "finished-source", false),
        ];
        annotate_waits(
            &mut activity,
            &[],
            &YieldedSources {
                tasks: tasks.to_vec(),
                wakes: Vec::new(),
                registrations: registrations.to_vec(),
            },
        );
        let waiting = activity["a"].liveness.waiting_on.clone().unwrap();
        assert_eq!(waiting.kind, AgentWaitKind::Source);
        assert_eq!(waiting.detail, "delegate-task-9");
        assert_eq!(waiting.since_ms, 7_000);
    }
}
