//! The pane feed: what one agent pane shows, in the kernel's words, so the web pane and the
//! provider terminals read one projection (docs/PROTOCOL.md section 3.3.2.1).
use std::collections::BTreeMap;

use crate::agent::AgentInstance;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
use crate::error::DaemonError;
use crate::local::{
    PaneCard, PaneCardChoice, PaneCardReply, PaneFeed, PaneIdentity, PaneNotice, PaneSettings,
    PaneState, PaneStripe, PersonInboxCard, PersonInboxCardBody,
};
use crate::runtime::projection::AgentRuntimeActivity;
use crate::session::{RuntimeInteraction, RuntimeInteractionInputKind, RuntimeSession};

/// A wait deadline at or past this is open-ended and shows no countdown.
const OPEN_ENDED_MS: u64 = 253_370_764_800_000;
/// A blocked task's own decision card; the card shows it, so no Blocked stripe does.
const BLOCKED_TASK_DECISION_PREFIX: &str = "task-blocked-";

pub(crate) struct PaneFeedSources<'a> {
    /// The Room as the caller may see it.
    pub session: &'a RuntimeSession,
    pub activity: &'a BTreeMap<String, AgentRuntimeActivity>,
    /// The caller's Inbox; only this agent's notices are read.
    pub inbox: &'a [PersonInboxCard],
    /// The web origin of the paired Cloud, when the kernel is paired.
    pub cloud_origin: Option<&'a str>,
}

pub(crate) fn project_pane_feed(
    sources: &PaneFeedSources<'_>,
    agent_id: &str,
) -> Result<PaneFeed, DaemonError> {
    let session = sources.session;
    let agent = session
        .agents()
        .iter()
        .find(|agent| agent.id() == agent_id)
        .ok_or_else(|| DaemonError::AgentNotInSession {
            session_id: session.id().to_string(),
            agent_id: agent_id.to_string(),
        })?;
    let room_name = session.alias().unwrap_or(session.id()).to_string();
    let agent_name = agent_name(agent);
    let link = sources
        .cloud_origin
        .map(|origin| pane_link(origin, session.id(), agent_id));
    let open_elsewhere = match &link {
        Some(link) => format!("Open in Chariox: {link}"),
        None => format!("Open the Room {room_name}, agent {agent_name}, in Chariox"),
    };
    let cards = session
        .active_interactions()
        .iter()
        .filter(|interaction| shown_in(interaction) == Some(agent_id))
        .map(|interaction| pane_card(interaction, agent_id, &open_elsewhere))
        .collect::<Vec<_>>();
    let config = crate::session::effective_agent_execution_config(session, Some(agent));
    let goal = running_goal(session, agent_id);
    Ok(PaneFeed {
        title: title_line(&room_name, &agent_name, cards.len(), goal.as_deref()),
        identity: PaneIdentity {
            room_id: session.id().to_string(),
            room_name,
            agent_name,
            agent_id: agent_id.to_string(),
            parents: parent_names(session, agent),
            project_id: session.project_id().to_string(),
            placement: agent
                .remote_execution()
                .map(|binding| binding.worker_machine_id.clone()),
        },
        settings: PaneSettings {
            provider: agent.provider().to_string(),
            account: agent.account_profile().map(str::to_owned),
            model: agent.model().map(str::to_owned),
            effort: agent.effort().map(str::to_owned),
            execution_mode: config.mode,
            permission_level: config.permission_level,
        },
        state: pane_state(sources.activity.get(agent_id), !cards.is_empty()),
        goal,
        stripes: stripes(session, agent),
        notices: notices(sources.inbox, session.id(), agent_id),
        cards,
        link,
    })
}

/// The web client opens a Room at an agent pane from this address; with no kernel named it
/// finds the Room in the person's lists.
fn pane_link(origin: &str, session_id: &str, agent_id: &str) -> String {
    let mut url = format!("{}/terminal", origin.trim_end_matches('/'));
    url.push('?');
    url.push_str(
        &url::form_urlencoded::Serializer::new(String::new())
            .append_pair("session", session_id)
            .append_pair("agent", agent_id)
            .finish(),
    );
    url
}

fn title_line(room: &str, agent: &str, cards: usize, goal: Option<&str>) -> String {
    let mut parts = vec![room.to_string(), agent.to_string()];
    match cards {
        0 => {}
        1 => parts.push("1 card waiting".into()),
        n => parts.push(format!("{n} cards waiting")),
    }
    parts.extend(goal.map(str::to_owned));
    parts.join(" · ")
}

/// The goal of the agent's latest task, while that task is not over.
fn running_goal(session: &RuntimeSession, agent_id: &str) -> Option<String> {
    let task = session
        .agent_tasks()
        .iter()
        .rev()
        .find(|task| task.agent_id == agent_id)?;
    let open = !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled);
    task.goal
        .as_ref()
        .filter(|_| open)
        .map(|goal| goal.objective.clone())
}

fn agent_name(agent: &AgentInstance) -> String {
    agent.alias().unwrap_or(agent.agent_ref()).to_string()
}

fn parent_names(session: &RuntimeSession, agent: &AgentInstance) -> Vec<String> {
    let mut names = Vec::new();
    let mut parent = agent.parent_agent_id();
    while let Some(id) = parent {
        let Some(found) = session.agents().iter().find(|agent| agent.id() == id) else {
            break;
        };
        if names.len() >= session.agents().len() {
            break;
        }
        names.push(agent_name(found));
        parent = found.parent_agent_id();
    }
    names.reverse();
    names
}

/// The pane a card shows in: the top parent's for a sub-agent's card, else its own agent's.
fn shown_in(interaction: &RuntimeInteraction) -> Option<&str> {
    interaction
        .shown_in_agent_id()
        .or_else(|| interaction.agent_id())
}

fn pane_state(activity: Option<&AgentRuntimeActivity>, has_cards: bool) -> PaneState {
    if has_cards {
        return PaneState::WaitingForAnswer;
    }
    match activity {
        Some(activity) if activity.busy || activity.active_prompt_count > 0 => PaneState::Working,
        _ => PaneState::Idle,
    }
}

fn pane_card(
    interaction: &RuntimeInteraction,
    pane_agent_id: &str,
    open_elsewhere: &str,
) -> PaneCard {
    let asked_by_other = interaction
        .agent_id()
        .is_some_and(|asker| asker != pane_agent_id);
    PaneCard {
        id: interaction.id().to_string(),
        asker_name: asked_by_other
            .then(|| interaction.asker_name().map(str::to_owned))
            .flatten(),
        title: interaction.title().map(str::to_owned),
        message: interaction.message().to_string(),
        choices: interaction
            .choices()
            .iter()
            .map(|choice| PaneCardChoice {
                id: choice.id().to_string(),
                label: choice.label().to_string(),
            })
            .collect(),
        reply: interaction.custom_choice().map(|custom| PaneCardReply {
            id: custom.id().to_string(),
            label: custom.label().to_string(),
            placeholder: custom.placeholder().map(str::to_owned),
            secret: custom.input_kind() == RuntimeInteractionInputKind::Secret,
        }),
        text_terminal: text_terminal_answers(interaction),
        requested_at_ms: interaction.requested_at_ms(),
        open_elsewhere: open_elsewhere.to_string(),
    }
}

/// A text terminal draws choices and a typed reply. Everything that needs a page, a passkey,
/// a payment sheet, a file chooser or a verified App frame is answered in the pane.
pub(crate) fn text_terminal_answers(interaction: &RuntimeInteraction) -> bool {
    let needs_pane = interaction.view().is_some()
        || interaction.handoff().is_some()
        || interaction.payment().is_some()
        || interaction.app_confirmation().is_some()
        || interaction.app_install().is_some()
        || interaction.provider_login().is_some()
        || interaction.sudo_request().is_some()
        || interaction
            .kernel_operation_id()
            .is_some_and(|operation| operation.starts_with("file_pick:"))
        || interaction
            .choices()
            .iter()
            .any(|choice| choice.requires_passkey());
    !needs_pane
}

fn stripes(session: &RuntimeSession, agent: &AgentInstance) -> Vec<PaneStripe> {
    let agent_id = agent.id();
    let mut stripes = Vec::new();
    if let Some(queued) = session.queued_prompts_for_agent(agent_id) {
        stripes.extend(queued.iter().map(|item| PaneStripe {
            id: format!("queue:{}", item.id()),
            kind: "Queued".into(),
            text: item.prompt().to_string(),
            at_ms: None,
        }));
    }
    let tasks = session
        .agent_tasks()
        .iter()
        .filter(|task| task.agent_id == agent_id)
        .collect::<Vec<_>>();
    let latest = tasks.last().map(|task| task.task_id.as_str());
    for task in &tasks {
        if Some(task.task_id.as_str()) == latest && task.state == ExecutionState::Cancelled {
            let kind = if task.goal.is_some() { "/goal" } else { "Task" };
            stripes.push(stripe(
                format!("task:{}", task.task_id),
                format!("{kind} · Stopped"),
                &task.reason,
                None,
            ));
            continue;
        }
        if let Some(goal) = &task.goal {
            let state = if goal.paused {
                "PAUSED".to_string()
            } else {
                execution_state_word(&task.state).to_uppercase()
            };
            stripes.push(stripe(
                format!("goal:{}", goal.id),
                format!("/goal · {state}"),
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
            let until = task
                .wait
                .as_ref()
                .and_then(|wait| wait.deadline_ms)
                .filter(|deadline| *deadline < OPEN_ENDED_MS);
            stripes.push(stripe(
                format!("wait:{}", task.task_id),
                "Blocked".into(),
                text,
                until,
            ));
        }
    }
    let waiting = tasks
        .iter()
        .filter(|task| task.state == ExecutionState::Waiting)
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    for wake in session.agent_wakes().iter().filter(|wake| {
        wake.agent_id == agent_id
            && matches!(wake.state.as_str(), "scheduled" | "starting" | "running")
            && !waiting.contains(&wake.task_id.as_str())
    }) {
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
        let due = wake.next_due_ms.filter(|due| *due < OPEN_ENDED_MS);
        stripes.push(stripe(format!("wake:{}", wake.id), kind.into(), text, due));
    }
    if let Some(limit) = agent.usage_limit_stop() {
        stripes.push(PaneStripe {
            id: format!("usage-limit:{}", limit.prompt_id),
            kind: "Usage limit".into(),
            text: format!("Usage limit reached on {}", limit.account_label),
            at_ms: limit.resumes_at_ms,
        });
    }
    stripes.retain(|stripe| !stripe.text.is_empty());
    stripes
}

fn stripe(id: String, kind: String, text: &str, at_ms: Option<u64>) -> PaneStripe {
    PaneStripe {
        id,
        kind,
        text: text.to_string(),
        at_ms,
    }
}

fn execution_state_word(state: &ExecutionState) -> &'static str {
    match state {
        ExecutionState::Working => "working",
        ExecutionState::Waiting => "waiting",
        ExecutionState::Blocked => "blocked",
        ExecutionState::Done => "done",
        ExecutionState::Cancelled => "cancelled",
    }
}

/// A delivery the kernel is reconciling is its own bookkeeping, nothing the person acts on.
fn is_delivery_bookkeeping(reason: &str) -> bool {
    let numbered = |rest: &str| {
        rest.split_once(' ')
            .is_some_and(|(n, _)| n.parse::<u64>().is_ok())
    };
    reason
        .strip_prefix("Unconfirmed delivery ")
        .is_some_and(|rest| rest.parse::<u64>().is_ok() || numbered(rest))
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

fn notices(inbox: &[PersonInboxCard], session_id: &str, agent_id: &str) -> Vec<PaneNotice> {
    let mut notices = inbox
        .iter()
        .filter(|card| card.session_id == session_id)
        .filter_map(|card| match &card.body {
            PersonInboxCardBody::Notice(notice) if notice.agent_id == agent_id && !notice.read => {
                Some(PaneNotice {
                    id: card.card_id.clone(),
                    title: notice.title.clone(),
                    body: notice.body.clone(),
                    at_ms: notice.created_at_ms,
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    notices.sort_by_key(|notice| std::cmp::Reverse(notice.at_ms));
    notices
}

#[cfg(test)]
#[path = "pane_feed_tests.rs"]
mod tests;
