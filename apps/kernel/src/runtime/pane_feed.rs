//! The pane feed: what one agent pane shows, in the kernel's words, so the web pane and the
//! provider terminals read one projection (docs/PROTOCOL.md section 3.3.2.1).
use std::collections::BTreeMap;

use crate::agent::AgentInstance;
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::error::DaemonError;
use crate::local::{
    PaneCard, PaneCardChoice, PaneCardReply, PaneFeed, PaneIdentity, PaneNotice, PaneSettings,
    PaneState, PersonInboxCard, PersonInboxCardBody,
};
use crate::runtime::projection::AgentRuntimeActivity;
use crate::session::{RuntimeInteraction, RuntimeInteractionInputKind, RuntimeSession};

pub(crate) struct PaneFeedSources<'a> {
    /// The Room as the caller may see it.
    pub session: &'a RuntimeSession,
    pub activity: &'a BTreeMap<String, AgentRuntimeActivity>,
    /// The Room's processes; a command of this agent waiting for memory or disk is a stripe.
    pub processes: &'a [crate::runtime::process_resources::SessionProcess],
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
        stripes: crate::runtime::pane_feed_stripes::stripes(session, agent, sources.processes),
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
