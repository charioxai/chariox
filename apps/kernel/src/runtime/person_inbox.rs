//! The person's Inbox on this kernel (docs/NOTIFICATION_CENTER_PLAN.md section 4): every card
//! that waits for the owner, from every Room they own here. A card is the Room's pending
//! interaction itself, so it closes everywhere when it is answered anywhere.
use crate::local::{LocalDaemonResponse, PersonInboxCard};
use crate::runtime::state::KernelRuntimeState;
use crate::session::RuntimeSession;

pub(crate) fn execute_person_inbox_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
) -> LocalDaemonResponse {
    let sessions = runtime_state.list_session_health_snapshots();
    let owned = sessions
        .iter()
        .filter(|session| runtime_state.acts_for_room_owner(session, caller_user_id));
    LocalDaemonResponse::PersonInbox {
        cards: person_inbox_cards(owned),
    }
}

/// Newest first.
fn person_inbox_cards<'a>(
    sessions: impl Iterator<Item = &'a RuntimeSession>,
) -> Vec<PersonInboxCard> {
    let mut cards = sessions
        .flat_map(|session| {
            session
                .named_active_interactions()
                .into_iter()
                .map(|interaction| PersonInboxCard {
                    session_id: session.id().to_owned(),
                    interaction,
                })
        })
        .collect::<Vec<_>>();
    cards.sort_by(|left, right| {
        right
            .interaction
            .requested_at_ms()
            .cmp(&left.interaction.requested_at_ms())
            .then_with(|| left.interaction.id().cmp(right.interaction.id()))
    });
    cards
}
