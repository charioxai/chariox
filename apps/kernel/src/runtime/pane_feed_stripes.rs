//! The stripes of an agent pane: every line of work the kernel holds for the agent, in the
//! kernel's words, so no client derives one (docs/PROTOCOL.md section 3.3.2.1).
use crate::agent::AgentInstance;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
use crate::local::PaneStripe;
use crate::runtime::process_resources::{
    SessionProcess, SessionProcessOwner, SessionProcessState, WaitReason, Waiting,
};
use crate::session::{
    AgentTriggerSource, MetaagentTaskStatus, RuntimeSession, WorkflowFailureKind,
    WorkflowResponsible, WorkflowRun, WorkflowRunOutcome, WorkflowScheduleTrigger,
    WorkflowWaitingOn,
};

/// A deadline at or past this is open-ended and shows no countdown.
const OPEN_ENDED_MS: u64 = 253_370_764_800_000;
/// A blocked task's own decision card; the card shows it, so no Blocked stripe does.
const BLOCKED_TASK_DECISION_PREFIX: &str = "task-blocked-";

pub(crate) fn stripes(
    session: &RuntimeSession,
    agent: &AgentInstance,
    processes: &[SessionProcess],
) -> Vec<PaneStripe> {
    let agent_id = agent.id();
    let mut stripes = waiting_commands(processes, agent_id);
    if let Some(queued) = session.queued_prompts_for_agent(agent_id) {
        stripes.extend(queued.iter().map(|item| {
            stripe(
                format!("queue:{}", item.id()),
                "Queued",
                item.prompt(),
                None,
            )
        }));
    }
    stripes.extend(triggers(session, agent_id));
    stripes.extend(tasks(session, agent_id));
    stripes.extend(wakes(session, agent_id));
    stripes.extend(metaagent_tasks(session, agent_id));
    stripes.extend(workflows(session, agent));
    if let Some(limit) = agent.usage_limit_stop() {
        stripes.push(PaneStripe {
            at_ms: limit.resumes_at_ms,
            ..stripe(
                format!("usage-limit:{}", limit.prompt_id),
                "Usage limit",
                &format!("Usage limit reached on {}", limit.account_label),
                None,
            )
        });
    }
    stripes.retain(|stripe| !stripe.text.is_empty());
    stripes
}

fn stripe(id: String, kind: &str, text: &str, detail: Option<String>) -> PaneStripe {
    PaneStripe {
        id,
        kind: kind.to_string(),
        text: text.to_string(),
        detail,
        at_ms: None,
    }
}

/// Commands of this agent the kernel holds for memory or disk: held ones first, then paused,
/// each in its place in line.
fn waiting_commands(processes: &[SessionProcess], agent_id: &str) -> Vec<PaneStripe> {
    let mut waiting = processes
        .iter()
        .filter(
            |process| matches!(&process.owner, SessionProcessOwner::Agent(id) if id == agent_id),
        )
        .filter_map(|process| match &process.state {
            SessionProcessState::Held(wait) => Some((0, process, wait, true)),
            SessionProcessState::Paused(wait) => Some((1, process, wait, false)),
            _ => None,
        })
        .collect::<Vec<_>>();
    waiting.sort_by_key(|(order, process, wait, _)| (*order, wait.position, process.process.pid));
    waiting
        .into_iter()
        .map(|(_, process, wait, held)| {
            let command = if wait.command_line.is_empty() {
                &process.process.command
            } else {
                &wait.command_line
            };
            let mut text = format!("{command} · {}", command_wait(wait, held));
            if process.remote_refusal.as_deref() == Some("no_capacity") {
                text.push_str(" · no VM free");
            }
            stripe(
                format!("job:{}", process.process.process_id),
                "Queued",
                &text,
                None,
            )
        })
        .collect()
}

fn command_wait(wait: &Waiting, held: bool) -> String {
    let what = match wait.reason {
        WaitReason::Memory => "memory",
        WaitReason::Disk => "disk space",
    };
    if !held {
        return match wait.reason {
            WaitReason::Memory => "paused at the memory limit".into(),
            WaitReason::Disk => "waiting for disk space".into(),
        };
    }
    if wait.position <= 1 {
        format!("waiting for {what}, needs {}", gigabytes(wait.need_bytes))
    } else {
        format!("waiting for {what}, {} in line", ordinal(wait.position))
    }
}

fn gigabytes(bytes: u64) -> String {
    format!(
        "{} GB",
        ((bytes as f64) / f64::from(1u32 << 30)).round().max(1.0)
    )
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The agent's triggers. A workflow's responsible agent keeps one trigger the person never
/// lists or edits, so it has no stripe.
fn triggers<'a>(
    session: &'a RuntimeSession,
    agent_id: &'a str,
) -> impl Iterator<Item = PaneStripe> + 'a {
    session
        .agent_triggers()
        .iter()
        .filter(move |trigger| {
            trigger.agent_id() == agent_id && trigger.responsible_for().is_none()
        })
        .map(|trigger| {
            let id = format!("trigger:{}", trigger.id());
            let counts = if trigger.done() {
                "Done".to_string()
            } else if trigger.coalesced() > 0 {
                format!(
                    "{} fired · {} skipped",
                    trigger.fired(),
                    trigger.coalesced()
                )
            } else {
                format!("{} fired", trigger.fired())
            };
            match trigger.source() {
                AgentTriggerSource::Notification {
                    display_name,
                    source_available,
                    ..
                } => stripe(
                    id,
                    "Notification",
                    display_name,
                    (trigger.enabled() && !source_available).then(|| "Offline".to_string()),
                ),
                AgentTriggerSource::Http { options, .. } => {
                    let methods = options
                        .methods
                        .iter()
                        .filter_map(|method| {
                            serde_json::to_value(method)
                                .ok()
                                .and_then(|value| value.as_str().map(str::to_owned))
                        })
                        .collect::<Vec<_>>()
                        .join(" and ");
                    stripe(
                        id,
                        "HTTP",
                        &format!("{methods} {}", options.route),
                        Some(counts),
                    )
                }
                AgentTriggerSource::Schedule {
                    trigger: schedule,
                    next_run_at_ms,
                    ..
                } => {
                    let when = match schedule {
                        WorkflowScheduleTrigger::Cron {
                            expression,
                            timezone,
                        } => format!("{expression} · {timezone}"),
                        WorkflowScheduleTrigger::Interval { every_seconds } => {
                            format!("Every {every_seconds}s")
                        }
                    };
                    PaneStripe {
                        at_ms: (trigger.enabled() && !trigger.done()).then_some(*next_run_at_ms),
                        ..stripe(id, "Schedule", &when, Some(counts))
                    }
                }
            }
        })
}

fn tasks(session: &RuntimeSession, agent_id: &str) -> Vec<PaneStripe> {
    let tasks = session
        .agent_tasks()
        .iter()
        .filter(|task| task.agent_id == agent_id)
        .collect::<Vec<_>>();
    let latest = tasks.last().map(|task| task.task_id.as_str());
    let mut stripes = Vec::new();
    for task in &tasks {
        if Some(task.task_id.as_str()) == latest && task.state == ExecutionState::Cancelled {
            let kind = if task.goal.is_some() { "/goal" } else { "Task" };
            stripes.push(stripe(
                format!("task:{}", task.task_id),
                kind,
                &task.reason,
                Some("Stopped".into()),
            ));
            continue;
        }
        if let Some(goal) = &task.goal {
            let state = if goal.paused {
                "PAUSED"
            } else {
                execution_state_word(&task.state)
            };
            stripes.push(stripe(
                format!("goal:{}", goal.id),
                &format!("/goal · {state}"),
                &goal.objective,
                None,
            ));
        }
        if task.state == ExecutionState::Blocked
            && !is_delivery_bookkeeping(&task.reason)
            && !has_owner_card(session, agent_id, task)
        {
            let text = if task.reason.is_empty() {
                "Waiting for the next task event"
            } else {
                &task.reason
            };
            stripes.push(PaneStripe {
                at_ms: task
                    .wait
                    .as_ref()
                    .and_then(|wait| wait.deadline_ms)
                    .filter(|deadline| *deadline < OPEN_ENDED_MS),
                ..stripe(format!("wait:{}", task.task_id), "Blocked", text, None)
            });
        }
    }
    stripes
}

fn execution_state_word(state: &ExecutionState) -> &'static str {
    match state {
        ExecutionState::Working => "WORKING",
        ExecutionState::Waiting => "WAITING",
        ExecutionState::Blocked => "BLOCKED",
        ExecutionState::Done => "DONE",
        ExecutionState::Cancelled => "CANCELLED",
    }
}

/// A delivery the kernel is reconciling is its own bookkeeping, nothing the person acts on.
pub(crate) fn is_delivery_bookkeeping(reason: &str) -> bool {
    let numbered = |rest: &str| {
        rest.split(' ')
            .next()
            .is_some_and(|n| n.parse::<u64>().is_ok())
    };
    reason
        .strip_prefix("Unconfirmed delivery ")
        .is_some_and(numbered)
        || reason
            .strip_prefix("Delivery ")
            .and_then(|rest| rest.split_once(' '))
            .is_some_and(|(n, rest)| n.parse::<u64>().is_ok() && rest.starts_with("refusal clock"))
}

fn has_owner_card(session: &RuntimeSession, agent_id: &str, task: &AgentTaskExecution) -> bool {
    let decision_prefix = format!("{BLOCKED_TASK_DECISION_PREFIX}{}-", task.task_id);
    session.active_interactions().iter().any(|interaction| {
        let mine = interaction.agent_id() == Some(agent_id)
            || interaction.related_agent_id() == Some(agent_id);
        mine && (interaction.asking_task_id() == Some(task.task_id.as_str())
            || interaction.id().starts_with(&decision_prefix))
    })
}

/// A wake the agent armed shows while it is armed; one its waiting task already shows is that row.
fn wakes(session: &RuntimeSession, agent_id: &str) -> Vec<PaneStripe> {
    let waiting = session
        .agent_tasks()
        .iter()
        .filter(|task| task.agent_id == agent_id && task.state == ExecutionState::Waiting)
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    session
        .agent_wakes()
        .iter()
        .filter(|wake| {
            wake.agent_id == agent_id
                && matches!(wake.state.as_str(), "scheduled" | "starting" | "running")
                && !waiting.contains(&wake.task_id.as_str())
        })
        .map(|wake| {
            let kind = match (wake.kind.as_str(), wake.interval_ms) {
                ("timer", Some(_)) => "/wait-every",
                ("timer", None) => "/wait-in",
                _ => "Waiting",
            };
            let text = if wake.label.is_empty() {
                &wake.summary
            } else {
                &wake.label
            };
            PaneStripe {
                at_ms: wake.next_due_ms.filter(|due| *due < OPEN_ENDED_MS),
                ..stripe(format!("wake:{}", wake.id), kind, text, None)
            }
        })
        .collect()
}

fn metaagent_tasks(session: &RuntimeSession, agent_id: &str) -> Vec<PaneStripe> {
    let mut stripes = Vec::new();
    for task in session
        .metaagent_tasks()
        .iter()
        .filter(|task| task.metaagent_id() == agent_id)
    {
        let id = task.task_id();
        let status = match task.status() {
            MetaagentTaskStatus::Active => "ACTIVE",
            MetaagentTaskStatus::Paused => "PAUSED",
            MetaagentTaskStatus::Blocked => "BLOCKED",
            MetaagentTaskStatus::Completed => "COMPLETED",
            MetaagentTaskStatus::Aborted => "ABORTED",
        };
        stripes.push(stripe(
            format!("meta:{id}"),
            &format!("/meta · {status}"),
            task.task_markdown(),
            None,
        ));
        for (prefix, kind, text) in [
            ("meta-blocked", "Blocked", task.blocked_reason()),
            ("meta-plan", "Plan", Some(task.plan_markdown())),
            ("meta-completed", "Completed", task.completion_summary()),
        ] {
            if let Some(text) = text {
                stripes.push(stripe(format!("{prefix}:{id}"), kind, text, None));
            }
        }
    }
    stripes.extend(
        session
            .queued_metaagent_tasks()
            .iter()
            .filter(|task| task.metaagent_id() == agent_id)
            .map(|task| {
                stripe(
                    format!("queued-meta:{}", task.id()),
                    "/meta · queued",
                    task.task_markdown(),
                    None,
                )
            }),
    );
    stripes
}

/// A run of a workflow the person answers for that needs them, on the agent of the step that
/// could not go on; then every unfinished run with a step on this agent.
fn workflows(session: &RuntimeSession, agent: &AgentInstance) -> Vec<PaneStripe> {
    let agent_id = agent.id();
    let mut stripes = Vec::new();
    let mut needing_person = Vec::new();
    for workflow in session.workflows() {
        if !matches!(workflow.responsible(), WorkflowResponsible::Person) {
            continue;
        }
        let Some(run) = session
            .workflow_runs()
            .iter()
            .filter(|run| run.workflow_id() == workflow.id())
            .max_by_key(|run| (run.number(), run.created_at_ms()))
        else {
            continue;
        };
        let Some(outcome) = run.outcome().filter(|_| run.attention()) else {
            continue;
        };
        let Some(words) = attention_words(&outcome, agent) else {
            continue;
        };
        needing_person.push(run.id());
        let step = match &outcome {
            WorkflowRunOutcome::Waiting { node_id, .. } => node_id.as_str(),
            WorkflowRunOutcome::Failed { .. } => run.entry_node_id(),
        };
        if node_agent(session, run, step) == Some(agent_id) {
            stripes.push(stripe(
                format!("attention:{}:{}", workflow.id(), run.id()),
                "Workflow",
                &format!("Run #{} {words}", run.number()),
                None,
            ));
        }
    }
    for run in session.workflow_runs() {
        if run.status().is_terminal() || needing_person.contains(&run.id()) {
            continue;
        }
        let Some(workflow) = session
            .workflows()
            .iter()
            .find(|workflow| workflow.id() == run.workflow_id())
        else {
            continue;
        };
        let on_agent = workflow
            .nodes()
            .iter()
            .any(|node| node.agent_id() == agent_id)
            || run
                .runtime_agent_ids_by_node()
                .values()
                .any(|id| id == agent_id);
        if on_agent {
            let text = run
                .invocation_prompt()
                .or_else(|| workflow.alias())
                .unwrap_or_default();
            stripes.push(stripe(
                format!("workflow:{}", run.id()),
                "Workflow",
                text,
                None,
            ));
        }
    }
    stripes
}

fn node_agent<'a>(
    session: &'a RuntimeSession,
    run: &'a WorkflowRun,
    node_id: &str,
) -> Option<&'a str> {
    run.runtime_agent_ids_by_node()
        .get(node_id)
        .map(String::as_str)
        .or_else(|| {
            session
                .workflows()
                .iter()
                .find(|workflow| workflow.id() == run.workflow_id())?
                .nodes()
                .iter()
                .find(|node| node.id() == node_id)
                .map(|node| node.agent_id())
        })
}

/// What the stripe says after "Run #n"; none when the run asks nothing of the person.
fn attention_words(outcome: &WorkflowRunOutcome, agent: &AgentInstance) -> Option<String> {
    match outcome {
        WorkflowRunOutcome::Failed { reason, detail } => Some(match reason {
            WorkflowFailureKind::RunStopped => return None,
            WorkflowFailureKind::StepNeedsHelp if detail.is_empty() => {
                format!(
                    "failed: {} needs help",
                    agent.alias().unwrap_or(agent.agent_ref())
                )
            }
            WorkflowFailureKind::StepNeedsHelp => format!(
                "failed: {} needs help — {detail}",
                agent.alias().unwrap_or(agent.agent_ref())
            ),
            WorkflowFailureKind::ProviderFailure => "failed: a provider failed".into(),
            WorkflowFailureKind::OutputValidationFailed
            | WorkflowFailureKind::WorkflowRunOutputValidationFailed => {
                "failed: the output was not accepted".into()
            }
            WorkflowFailureKind::NodeTurnBudgetExhausted => {
                "failed: a step used all its turns".into()
            }
            WorkflowFailureKind::TurnStalled => "failed: a step stalled".into(),
            WorkflowFailureKind::TransportFailure => "failed: a connection failed".into(),
            WorkflowFailureKind::MissingAck | WorkflowFailureKind::MissingStructuredOutput => {
                "failed: a step did not report back".into()
            }
        }),
        WorkflowRunOutcome::Waiting { on, .. } => Some(format!(
            "waiting on {}",
            match on {
                WorkflowWaitingOn::ProviderProfileUsageExhausted => {
                    let provider = agent.provider();
                    let mut name = provider.chars();
                    match name.next() {
                        Some(first) => {
                            format!("{}{} usage limit", first.to_uppercase(), name.as_str())
                        }
                        None => "a usage limit".into(),
                    }
                }
                WorkflowWaitingOn::ProviderProfileUnauthenticated => "a provider sign-in".into(),
                WorkflowWaitingOn::ProviderProfileUnavailable
                | WorkflowWaitingOn::ProviderProfileUnverified => "a provider".into(),
                WorkflowWaitingOn::BlockedOnWorkspaceClaim => "a busy workspace".into(),
            }
        )),
    }
}

#[cfg(test)]
#[path = "pane_feed_stripes_tests.rs"]
mod tests;
