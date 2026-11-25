//! Admission for owner-facing interactions. RuntimeInteraction is also wire
//! data, so possessing a presentation does not authorize publishing it.
use super::*;
use crate::session::{RuntimeInteraction, RuntimeSession};

mod live_room_card;
pub(crate) use live_room_card::LiveRoomCard;

macro_rules! card_kinds {
    ($($variant:ident => [$name:literal, $event:literal, $live:literal, $answers:literal, $ends:literal, $restart:literal]),+ $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        pub(crate) enum CardKind {
            $($variant),+
        }

        impl CardKind {
            const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
        }

        #[cfg(test)]
        const TABLE: &str = concat!(
            "| Group | Trigger | Live state | Answerer | Retirement | Cold restart |\n",
            "| --- | --- | --- | --- | --- | --- |\n",
            $("| ", $name, " | ", $event, " | ", $live, " | ", $answers, " | ", $ends, " | ", $restart, " |\n",)+
        );
    };
}

card_kinds! {
    Task => ["Task", "An owner blocker or task question is admitted", "Current blocked revision or unanswered wake bound to its task", "Eligible holder or Room owner", "Answer, timeout, cancellation or supersession", "Recheck the ledger and rearm current blockers"],
    NativePermissionInput => ["Native permission or input", "Provider request, forwarded request or live popup", "Live requester and matching provider turn or lease where applicable", "Eligible holder or Room owner under native policy", "Answer, timeout or end of the caller or turn", "Requires a fresh request"],
    ProviderLogin => ["Provider login", "Login, logout, recovery, enrollment or account copy needs input", "Current provider-account operation and requester", "Authorized account or kernel owner", "Answer, process exit, timeout or cancellation", "Only a recovered operation can request input again"],
    VaultCredentialPayment => ["Vault, credential or payment", "An exact secret, unlock, credential or payment operation needs input", "Current requester and exact operation, with secret delivery scoped to its destination", "Authorized owner", "Answer, completion, timeout or abandonment", "Requires a new live waiter"],
    RuntimeGrant => ["Runtime grant", "An operation needs a missing permission", "Current requesting operation and ungranted scope", "Authorized owner or eligible holder", "Answer, existing grant, timeout or abandonment", "Requires a fresh request"],
    App => ["App", "An App install, publish, validation, deployment or host operation needs consent", "Current pending operation and eligible App revision or caller", "Authorized App owner or eligible holder", "Answer, expiry, revocation or abandonment", "Revalidate still-pending durable operations"],
    OwnerControl => ["Owner control", "An owner decision, access request, hand-off or usage stop needs a response", "Current operation, authorized owner, and applicable live requester or target", "Authorized owner or eligible holder under the operation's policy", "Answer, timeout, supersession or abandonment", "Revalidate durable hand-offs and usage stops"],
}

/// A producer selects an existing trigger group. The presentation remains
/// untrusted until the normal registration path has validated its authority.
pub(in crate::runtime::state) struct CardRequest(pub CardKind, pub RuntimeInteraction);

fn stale(kind: CardKind, id: &str) -> DaemonError {
    tracing::error!(card_group = kind.name(), interaction_id = %id, "card refused: no live requester");
    interaction_error("The requesting operation ended before its card could open")
}

pub(super) fn card_presentation(card: &LiveRoomCard) -> &RuntimeInteraction {
    card.presentation()
}

pub(super) fn admit_scoped_card(
    kind: CardKind,
    interaction: &RuntimeInteraction,
    responder: &tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
) -> Result<(), DaemonError> {
    if responder.is_closed() {
        return Err(stale(kind, interaction.id()));
    }
    Ok(())
}

impl KernelRuntimeOwnedState {
    /// Called after the domain has admitted the request and while registration
    /// owns the session mutation. A renderable card cannot leave this boundary
    /// unless the requester is still live.
    pub(super) fn admit_room_card(
        &self,
        kind: CardKind,
        interaction: RuntimeInteraction,
        pending: &super::super::PendingInteraction,
        session: &RuntimeSession,
        worker_live: bool,
    ) -> Result<LiveRoomCard, DaemonError> {
        if session.status() == crate::session::SessionStatus::Ended
            || pending.nobody_waits()
            || !worker_live
            || !self.agent_interaction_turn_is_live(pending, session)
        {
            return Err(stale(kind, interaction.id()));
        }
        Ok(LiveRoomCard::admitted(interaction))
    }
}

#[cfg(test)]
#[test]
fn generated_card_table_covers_each_kind() {
    assert_eq!(
        include_str!("../../../../../../docs/RUNTIME_CARD_RULES.md"),
        TABLE
    );
}
