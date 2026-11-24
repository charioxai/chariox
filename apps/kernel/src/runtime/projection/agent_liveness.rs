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
    /// Every event the agent's waiting task would wake on, oldest first: what a client lists
    /// as "Waiting on …". `waiting_on` names only the oldest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting_events: Vec<AgentWaitEvent>,
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
                due_at_ms: None,
            }),
            waiting_events: Vec::new(),
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
    /// When a timer is due; absent for anything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at_ms: Option<u64>,
}

/// One event a waiting task is registered for: an armed wake, or a source (an agent, a workflow
/// run) that has not reported yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWaitEvent {
    /// The wake id for a wake, the registration id for any other source.
    pub id: String,
    pub kind: AgentWaitKind,
    /// What the event is, in plain words: "sub-agent sub-agent-1", "timer: check the build".
    pub label: String,
    pub since_ms: u64,
    /// When a timer is due; absent for anything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at_ms: Option<u64>,
    /// The task that waits, and when its wait began: the owner ends the whole wait with these.
    pub task_id: String,
    pub wait_started_at_ms: u64,
    /// Set when the owner can end this one event alone (a wake, or the follow of a peer): the id to
    /// send with `CancelAgentWake`. A sub-agent's end and a workflow run end on their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_wake_id: Option<String>,
}

/// How a source the agent waits on is named in the pane: the agents and workflow runs of its Room.
#[derive(Debug, Clone, Default)]
pub(crate) struct SourceLabels(BTreeMap<String, String>);

impl SourceLabels {
    /// A child agent reads "sub-agent <name>", any other agent "agent <name>", a run "workflow <name>".
    pub(crate) fn of_session(session: &crate::session::RuntimeSession) -> Self {
        let mut labels = BTreeMap::new();
        for agent in session.agents() {
            let name = agent.alias().unwrap_or_else(|| agent.agent_ref());
            let kind = if agent.parent_agent_id().is_some() {
                "sub-agent"
            } else {
                "agent"
            };
            labels.insert(agent.id().to_owned(), format!("{kind} {name}"));
        }
        for run in session.workflow_runs() {
            let name = session
                .workflows()
                .iter()
                .find(|workflow| workflow.id() == run.workflow_id())
                .and_then(|workflow| workflow.alias())
                .unwrap_or_else(|| run.workflow_id());
            labels.insert(run.id().to_owned(), format!("workflow {name}"));
        }
        Self(labels)
    }

    #[cfg(test)]
    fn from_pairs<const N: usize>(pairs: [(&str, &str); N]) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(id, label)| (id.to_owned(), label.to_owned()))
                .collect(),
        )
    }

    fn of(&self, source_id: &str) -> String {
        self.0
            .get(source_id)
            .cloned()
            .unwrap_or_else(|| format!("task {source_id}"))
    }
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
    /// A decision card the agent's turn is held on: the owner's, or one an agent holds.
    Card,
}

/// The state `agent children` and the session agent inspection name for an
/// agent. One that yielded on a job, timer or source is waiting, not idle: its
/// `liveness.waiting_on` says on what and since when.
pub(crate) fn agent_state_label(
    agent: &crate::agent::AgentInstance,
    activity: Option<&AgentRuntimeActivity>,
) -> &'static str {
    use crate::agent::AgentState;
    let yielded = activity.is_some_and(|activity| {
        activity.status == AgentRuntimeStatus::Idle && activity.liveness.waiting_on.is_some()
    });
    match agent.state() {
        AgentState::Idle | AgentState::Focused if yielded => "waiting",
        AgentState::Idle => "idle",
        AgentState::Working => "working",
        AgentState::Focused => "focused",
        AgentState::Error => "error",
    }
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
    labels: &SourceLabels,
) {
    for (agent_id, activity) in activity.iter_mut() {
        let waiting = command_wait(agent_id, processes).or_else(|| yielded_wait(agent_id, yielded));
        if let Some(waiting) = waiting {
            activity.liveness.waiting_on = Some(waiting);
        }
        activity.liveness.waiting_events = yielded_events(agent_id, yielded, labels);
    }
}

/// Names the decision cards each agent's turn is held on, by the title the card shows and never
/// by its fields. A card the user holds reads "your answer on '<title>'"; one an ancestor agent
/// holds reads as that agent's. The oldest of all a wait names is `waiting_on`.
pub(crate) fn annotate_cards(
    activity: &mut BTreeMap<String, AgentRuntimeActivity>,
    session: &crate::session::RuntimeSession,
    held_by_agent: &std::collections::HashSet<String>,
) {
    for interaction in session.active_interactions() {
        let Some(agent_id) = interaction.agent_id() else {
            continue;
        };
        let Some(activity) = activity.get_mut(agent_id) else {
            continue;
        };
        let title = interaction.title().map_or_else(
            || "a decision".to_owned(),
            |title| format!("'{}'", title.trim()),
        );
        let label = if held_by_agent.contains(interaction.id()) {
            format!("the answer of the agent that holds {title}")
        } else {
            format!("your answer on {title}")
        };
        let since_ms = interaction.requested_at_ms();
        activity.liveness.waiting_events.push(AgentWaitEvent {
            id: interaction.id().to_owned(),
            kind: AgentWaitKind::Card,
            label,
            since_ms,
            due_at_ms: None,
            task_id: String::new(),
            wait_started_at_ms: since_ms,
            cancel_wake_id: None,
        });
        activity
            .liveness
            .waiting_events
            .sort_by(|a, b| a.since_ms.cmp(&b.since_ms).then_with(|| a.id.cmp(&b.id)));
        if activity
            .liveness
            .waiting_on
            .as_ref()
            .is_none_or(|on| on.kind == AgentWaitKind::Provider || since_ms < on.since_ms)
        {
            activity.liveness.waiting_on = Some(AgentWaitingOn {
                kind: AgentWaitKind::Card,
                detail: title,
                resource: None,
                since_ms,
                due_at_ms: None,
            });
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
        due_at_ms: None,
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
        due_at_ms: wake.next_due_ms.filter(|_| wake.kind == "timer"),
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
                due_at_ms: None,
            })
    });
    wake_waits
        .chain(source_waits)
        .min_by_key(|wait| wait.since_ms)
}

/// Everything the agent's waiting task is registered for, oldest first: its live wakes, and the
/// other sources (a sub-agent, a workflow run) that have not reported yet.
fn yielded_events(
    agent_id: &str,
    yielded: &YieldedSources,
    labels: &SourceLabels,
) -> Vec<AgentWaitEvent> {
    let YieldedSources {
        tasks,
        wakes,
        registrations,
    } = yielded;
    let mut events = Vec::new();
    for task in tasks
        .iter()
        .filter(|task| task.agent_id == agent_id && task.state == ExecutionState::Waiting)
    {
        let wait_started_at_ms = task
            .wait
            .as_ref()
            .map_or(task.last_progress_at_ms, |wait| wait.started_at_ms);
        let registered = task
            .wait
            .as_ref()
            .map_or(&[][..], |wait| &wait.registration_ids[..]);
        let live_wakes = wakes
            .iter()
            .filter(|wake| wake.task_id == task.task_id)
            .filter(|wake| matches!(wake.state.as_str(), "scheduled" | "starting" | "running"))
            .collect::<Vec<_>>();
        for wake in &live_wakes {
            let (kind, noun) = match wake.kind.as_str() {
                "process" => (AgentWaitKind::BackgroundCommand, "command"),
                "terminal" => (AgentWaitKind::BackgroundCommand, "terminal"),
                "timer" => (AgentWaitKind::Timer, "timer"),
                _ => (AgentWaitKind::Source, "event"),
            };
            let text = if wake.label.is_empty() {
                wake.command.join(" ")
            } else {
                wake.label.clone()
            };
            events.push(AgentWaitEvent {
                id: wake.id.clone(),
                kind,
                label: if text.is_empty() {
                    noun.to_owned()
                } else {
                    format!("{noun} {text}")
                },
                since_ms: wake.created_at_ms,
                due_at_ms: wake.next_due_ms.filter(|_| wake.kind == "timer"),
                task_id: task.task_id.clone(),
                wait_started_at_ms,
                cancel_wake_id: Some(wake.id.clone()),
            });
        }
        for registration in registrations.iter().filter(|registration| {
            registration.live
                && registration.task_id == task.task_id
                && registered.contains(&registration.id)
                && !live_wakes
                    .iter()
                    .any(|wake| wake.registration_id == registration.id)
        }) {
            events.push(AgentWaitEvent {
                id: registration.id.clone(),
                kind: AgentWaitKind::Source,
                // A delegation binds its completion source to the child's task: its obligation names the child agent.
                label: labels.of(task
                    .obligations
                    .iter()
                    .find(|o| {
                        o.completion_task_id.as_deref() == Some(registration.source_id.as_str())
                    })
                    .and_then(|o| o.resource_id.as_deref())
                    .unwrap_or(&registration.source_id)),
                since_ms: wait_started_at_ms,
                due_at_ms: None,
                task_id: task.task_id.clone(),
                wait_started_at_ms,
                // A completion source covers an obligation the task still owes; only a follow of a
                // peer ends alone.
                cancel_wake_id: (registration.peer && registration.obligation_id.is_none())
                    .then(|| registration.id.clone()),
            });
        }
    }
    events.sort_by(|a, b| a.since_ms.cmp(&b.since_ms).then_with(|| a.id.cmp(&b.id)));
    events
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
        // Only waiting tasks yield on sources; the Room's finished turns are not read.
        let tasks = store.agent_waiting_tasks(room).unwrap_or_default();
        if tasks.is_empty() {
            return Self::default();
        }
        let wakes = store.agent_waiting_wakes(room).unwrap_or_default();
        let registrations = tasks
            .iter()
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
mod history_load_tests;

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
        annotate_waits(
            &mut activity,
            &processes,
            &YieldedSources::default(),
            &SourceLabels::default(),
        );
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
            &SourceLabels::default(),
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
            &SourceLabels::default(),
        );
        let waiting = activity["a"].liveness.waiting_on.clone().unwrap();
        assert_eq!(waiting.kind, AgentWaitKind::Source);
        assert_eq!(waiting.detail, "delegate-task-9");
        assert_eq!(waiting.since_ms, 7_000);
    }

    /// Reviewer P2: a parent waiting on a dispatched helper reads "sub-agent <name>", not the child's opaque task id the
    /// delegation binding rewrites the completion source to.
    #[test]
    fn a_parent_waiting_on_a_bound_delegation_names_the_child_agent() {
        use crate::durable_state::agent_lifecycle::Operation;
        let root =
            std::env::temp_dir().join(format!("chariox-waits-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let store =
            crate::durable_state::DurableKernelStateStore::open_owned(root.join("state.sqlite"))
                .unwrap();
        let apply = |op| store.agent_lifecycle(op).unwrap();
        let begin = |agent: &str, prompt: &str| {
            apply(Operation::Begin {
                owner: "owner".into(),
                room: "room".into(),
                agent: agent.into(),
                prompt: prompt.into(),
                run: Some("run".into()),
                now: 1,
            })
        };
        begin("parent", "p");
        apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: "room".into(),
            agent: "parent".into(),
            prompt: "p".into(),
            run: Some("run".into()),
            id: "obligation".into(),
            kind: "delegate".into(),
            resource: Some("child".into()),
            now: 1,
        });
        apply(Operation::DispatchReceipt {
            id: "obligation".into(),
            accepted: true,
            resource: Some("child".into()),
        });
        apply(Operation::Subscribe {
            task: "p".into(),
            prompt: "p".into(),
            registration: Registration {
                id: "reg".into(),
                task_id: "p".into(),
                source_id: "child".into(),
                obligation_id: Some("obligation".into()),
                source_cursor: 0,
                live: true,
                peer: false,
            },
        });
        begin("child", "child-task");
        apply(Operation::BindDelegate {
            parent_task: "p".into(),
            child_task: "child-task".into(),
        });
        apply(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec!["reg".into()],
            cursor: 0,
            deadline: Some(60_000),
            reason: "child result".into(),
            now: 2,
        });
        apply(Operation::Settle {
            room: "room".into(),
            agent: "parent".into(),
            prompt: "p".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 3,
        });
        let mut activity = BTreeMap::from([("parent".to_string(), idle_activity())]);
        annotate_waits(
            &mut activity,
            &[],
            &YieldedSources::of_room(&store, "room"),
            &SourceLabels::from_pairs([("child", "sub-agent builder")]),
        );
        let labels = activity["parent"]
            .liveness
            .waiting_events
            .iter()
            .map(|event| event.label.as_str())
            .collect::<Vec<_>>();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(labels, vec!["sub-agent builder"]);
    }

    /// The owner removes one event alone wherever the kernel can end it: a wake of any kind and the
    /// follow of another agent. A sub-agent's end (a source an obligation covers) has no such control.
    #[test]
    fn the_owner_can_remove_a_process_and_a_peer_follow_but_not_a_sub_agents_end() {
        let mut activity = BTreeMap::from([("a".to_string(), idle_activity())]);
        let tasks = [waiting_task("a", "t1", &["reg-child", "reg-peer"], 7_000)];
        let mut peer = registration("t1", "reg-peer", "another-agent", true);
        peer.peer = true;
        let registrations = [registration("t1", "reg-child", "child-1", true), peer];
        let wakes = [wake("a", "t1", "process", "running", 9_000)];
        annotate_waits(
            &mut activity,
            &[],
            &YieldedSources {
                tasks: tasks.to_vec(),
                wakes: wakes.to_vec(),
                registrations: registrations.to_vec(),
            },
            &SourceLabels::from_pairs([("child-1", "sub-agent sub-agent-1")]),
        );
        let events = &activity["a"].liveness.waiting_events;
        assert_eq!(
            events
                .iter()
                .map(|event| (event.id.as_str(), event.cancel_wake_id.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("reg-child", None),
                ("reg-peer", Some("reg-peer")),
                ("wake-9000", Some("wake-9000")),
            ]
        );
    }

    /// Miguel 11-16: a waiting agent says what would wake it: every armed wake and every source it is registered for.
    #[test]
    fn a_waiting_agent_lists_every_event_it_waits_on_oldest_first() {
        let mut activity = BTreeMap::from([("a".to_string(), idle_activity())]);
        let tasks = [waiting_task(
            "a",
            "t1",
            &["reg-child", "reg-run", "reg-done", "r"],
            7_000,
        )];
        let registrations = [
            registration("t1", "reg-child", "child-1", true),
            registration("t1", "reg-run", "run-9", true),
            registration("t1", "reg-done", "gone", false),
            registration("t1", "r", "wake-9000", true),
        ];
        let wakes = [
            wake("a", "t1", "timer", "scheduled", 9_000),
            wake("a", "t1", "process", "exited", 8_000),
        ];
        let labels = SourceLabels::from_pairs([
            ("child-1", "sub-agent sub-agent-1"),
            ("run-9", "workflow Review"),
        ]);
        annotate_waits(
            &mut activity,
            &[],
            &YieldedSources {
                tasks: tasks.to_vec(),
                wakes: wakes.to_vec(),
                registrations: registrations.to_vec(),
            },
            &labels,
        );
        let events = &activity["a"].liveness.waiting_events;
        assert_eq!(
            events
                .iter()
                .map(|event| (
                    event.label.as_str(),
                    event.since_ms,
                    event.cancel_wake_id.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("sub-agent sub-agent-1", 7_000, None),
                ("workflow Review", 7_000, None),
                ("timer build", 9_000, Some("wake-9000")),
            ],
            "the live registrations and the live timer; an ended source or wake is not waited on"
        );
        assert!(events
            .iter()
            .all(|event| event.task_id == "t1" && event.wait_started_at_ms == 7_000));
    }
}
