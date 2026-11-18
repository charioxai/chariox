use super::*;

/// The owner's Inbox on this kernel (docs/NOTIFICATION_CENTER_PLAN.md section 4). Read through
/// the `person_inbox` kernel resource, whose snapshots are its change events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetPersonInboxRequest;

/// One card for the person, from one Room of theirs on this kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInboxCard {
    /// The interaction id of a needs-you card, the notice id of a for-your-information one.
    pub card_id: String,
    pub session_id: String,
    /// Hidden from the Inbox until then; a snoozed needs-you card still waits in its Room.
    pub snoozed_until_ms: Option<u64>,
    pub body: PersonInboxCardBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonInboxCardBody {
    /// A pending interaction of the Room, as the Room shows it, so answering it here or in the
    /// Room is the same `RespondToInteraction`.
    NeedsYou {
        interaction: Box<crate::session::RuntimeInteraction>,
    },
    /// What an agent told the person with `chariox.notify_person`; nothing waits on it.
    Notice(PersonInboxNotice),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInboxNotice {
    pub agent_id: String,
    /// The agent's name when the Inbox is read; none once the agent is gone.
    pub agent_name: Option<String>,
    pub title: String,
    pub body: Option<String>,
    /// A URL or a Chariox target the person can open.
    pub link: Option<String>,
    pub priority: PersonInboxPriority,
    pub created_at_ms: u64,
    pub read: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonInboxPriority {
    #[default]
    Normal,
    /// Sorts first.
    High,
}

/// Whose cards a mute silences: one agent of one Room.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInboxOrigin {
    pub session_id: String,
    pub agent_id: String,
}

/// Read applies to notices only; snooze to any card. The new state comes back as `PersonInbox`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetInboxCardStateRequest {
    pub card_ids: Vec<String>,
    pub change: InboxCardChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum InboxCardChange {
    Read {
        read: bool,
    },
    /// `None` brings the cards back now.
    Snooze {
        until_ms: Option<u64>,
    },
}

/// A muted origin's notices arrive read. Needs-you cards cannot be muted: an agent waits on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetInboxMuteRequest {
    pub origin: PersonInboxOrigin,
    pub muted: bool,
}
