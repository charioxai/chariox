use super::*;

/// The owner's Inbox on this kernel (docs/NOTIFICATION_CENTER_PLAN.md section 4). Read through
/// the `person_inbox` kernel resource, whose snapshots are its change events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetPersonInboxRequest;

/// One card for the person: a pending interaction of one of their Rooms, as the Room shows it,
/// so answering it here or in the Room is the same `RespondToInteraction`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInboxCard {
    pub session_id: String,
    pub interaction: crate::session::RuntimeInteraction,
}
