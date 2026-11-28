//! Admission for owner-facing interactions. RuntimeInteraction is also wire
//! data, so possessing a presentation does not authorize publishing it.
use super::*;
use crate::session::{RuntimeInteraction, RuntimeSession};

mod live_room_interaction;
pub(crate) use live_room_interaction::LiveRoomInteraction;

#[derive(Clone, Copy, Debug)]
pub(crate) enum InteractionSurface {
    Card(&'static str),
    PaneStatus,
    Hidden,
}

impl InteractionSurface {
    pub(crate) const fn named(name: &'static str) -> Self {
        Self::Card(name)
    }
    pub(crate) fn native(interaction: &RuntimeInteraction) -> Self {
        let id = interaction.id();
        if id.starts_with("opencode-permission-") {
            Self::named("opencode_permission")
        } else if id.starts_with("codex-command-approval-")
            || id.starts_with("codex-exec-command-approval-")
        {
            Self::named("codex_command_approval")
        } else if id.starts_with("codex-file-change-approval-")
            || id.starts_with("codex-apply-patch-approval-")
        {
            Self::named("codex_file_change_approval")
        } else if id.starts_with("claude-permission-") {
            Self::named("claude_permission_hook")
        } else {
            Self::named("provider_native_permission")
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Card(name) => name,
            Self::PaneStatus => "pane status",
            Self::Hidden => "hidden operation",
        }
    }
    pub(super) const fn is_card(self) -> bool {
        matches!(self, Self::Card(_))
    }
    pub(super) const fn is_visible(self) -> bool {
        !matches!(self, Self::Hidden)
    }
    pub(super) fn present(self, interaction: RuntimeInteraction) -> RuntimeInteraction {
        match self {
            Self::Card(name) => interaction.with_card_type(name),
            Self::PaneStatus => interaction.as_pane_status(),
            Self::Hidden => interaction.hidden(),
        }
    }
}

/// A producer names its individual type. The presentation remains
/// untrusted until the normal registration path has validated its authority.
pub(in crate::runtime::state) struct InteractionRequest(
    pub InteractionSurface,
    pub RuntimeInteraction,
);

pub(super) fn require_registered_type(interaction: &RuntimeInteraction) -> Result<(), DaemonError> {
    if crate::session::runtime_card_class(interaction.card_type()).is_none() {
        tracing::error!(
            card_type = interaction.card_type(),
            interaction_id = interaction.id(),
            "card refused: unregistered type"
        );
        return Err(interaction_error("This card type is not registered. Keep the operation in its existing pane until its individual type is approved."));
    }
    if interaction.card_type() == "Question"
        && (interaction.kind() != crate::session::RuntimeInteractionKind::Choice
            || interaction.custom_choice().is_none())
    {
        return Err(interaction_error(
            "A Question has a free-text answer, with optional choices.",
        ));
    }
    Ok(())
}

fn stale(kind: InteractionSurface, id: &str) -> DaemonError {
    tracing::error!(presentation_type = kind.name(), interaction_id = %id, "card refused: no live requester");
    interaction_error("The requesting operation ended before its card could open")
}

pub(super) fn interaction_presentation(card: &LiveRoomInteraction) -> &RuntimeInteraction {
    card.presentation()
}

pub(super) fn admit_scoped_card(
    kind: InteractionSurface,
    interaction: &RuntimeInteraction,
    responder: &tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
) -> Result<(), DaemonError> {
    if kind.is_card() {
        require_registered_type(interaction)?;
    }
    if responder.is_closed() {
        return Err(stale(kind, interaction.id()));
    }
    Ok(())
}

impl KernelRuntimeOwnedState {
    /// Called after the domain has admitted the request and while registration
    /// owns the session mutation. A renderable card cannot leave this boundary
    /// unless the requester is still live.
    pub(super) fn admit_room_interaction(
        &self,
        kind: InteractionSurface,
        interaction: RuntimeInteraction,
        pending: &super::super::PendingInteraction,
        session: &RuntimeSession,
        worker_live: bool,
    ) -> Result<LiveRoomInteraction, DaemonError> {
        if kind.is_card() {
            require_registered_type(&interaction)?;
        }
        if session.status() == crate::session::SessionStatus::Ended
            || pending.nobody_waits()
            || !worker_live
            || !self.agent_interaction_turn_is_live(pending, session)
        {
            return Err(stale(kind, interaction.id()));
        }
        Ok(LiveRoomInteraction::admitted(interaction))
    }
}

#[cfg(test)]
#[test]
fn documented_types_match_the_authoritative_list() {
    let entries: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../../../docs/runtime-card-types.json"
    ))
    .unwrap();
    let documentation = include_str!("../../../../../../docs/RUNTIME_CARD_RULES.md");
    for entry in entries {
        assert!(documentation.contains(entry["type"].as_str().unwrap()));
    }
}
