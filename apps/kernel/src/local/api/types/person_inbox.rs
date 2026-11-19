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
    /// A run of a workflow the person answers for failed or holds on something. It leaves when
    /// the person opens the run or a newer run starts.
    Workflow(PersonInboxWorkflowRun),
    /// A subscription in the person's Room stopped delivering; it leaves when the subscription
    /// is fixed or removed.
    Flag(PersonInboxFlag),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInboxWorkflowRun {
    pub workflow_id: String,
    pub workflow_name: String,
    pub run_id: String,
    pub run_number: u32,
    pub outcome: crate::session::WorkflowRunOutcome,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInboxFlag {
    pub subscription_id: String,
    /// None once the subscriber is gone.
    pub subscriber_name: Option<String>,
    /// None once the source is gone.
    pub source_name: Option<String>,
    pub health: NotificationSubscriptionHealth,
    /// When the flag was raised, when the kernel knows it.
    pub at_ms: Option<u64>,
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
    /// 0 for one notice. n when the agent sent more notices than the rate limit allows and this
    /// card stands for the n notices past it: Noisy. `title` then reads "<agent> sent n more
    /// notifications" and `body` names the latest one.
    pub folded: u32,
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
