use super::*;

/// What one agent pane shows, built by the kernel so every client says the same words
/// (docs/PROTOCOL.md section 3.3.2.1). Read once with `GetPaneFeed`, or watched as the
/// `pane_feed` kernel resource with params `{ session_id, agent_id }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetPaneFeedRequest {
    pub session_id: String,
    pub agent_id: String,
}

/// The settings a provider's own terminal changed (its model picker, its mode switch), told to
/// the kernel so every client shows them. Only an agent a provider terminal runs takes it;
/// absent fields are unchanged. Answered with `PaneFeed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportPaneSettingsRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneFeed {
    /// One line for a terminal title: "Room · agent · 2 cards waiting · goal". It stays while
    /// a card waits, so a terminal with no status line still shows it until it is settled.
    pub title: String,
    pub identity: PaneIdentity,
    pub settings: PaneSettings,
    pub state: PaneState,
    /// The objective of the agent's running goal.
    pub goal: Option<String>,
    /// The cards shown in this pane: the agent's own and its sub-agents', oldest first.
    pub cards: Vec<PaneCard>,
    pub stripes: Vec<PaneStripe>,
    /// The person's unread notices from this agent, newest first.
    pub notices: Vec<PaneNotice>,
    /// Opens this pane in the web client of the Cloud the kernel is paired with; none when
    /// the kernel has no pairing. It names the Room and agent, never a card's content.
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneIdentity {
    pub room_id: String,
    pub room_name: String,
    pub agent_id: String,
    pub agent_name: String,
    /// The names of the agents above a sub-agent, top first; empty for a top-level agent.
    pub parents: Vec<String>,
    pub project_id: String,
    /// The machine the agent runs on when it is not this kernel's own.
    pub placement: Option<String>,
}

/// The settings as the kernel records them; mode and permissions with the Room's defaults applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSettings {
    pub provider: String,
    pub account: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub execution_mode: crate::provider::AgentExecutionMode,
    pub permission_level: crate::provider::AgentPermissionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneState {
    Idle,
    Working,
    /// A card of this pane waits for the person.
    WaitingForAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneCard {
    /// The interaction id; every answer is `RespondToInteraction` with it.
    pub id: String,
    /// Who asks, when it is not this pane's agent.
    pub asker_name: Option<String>,
    pub title: Option<String>,
    pub message: String,
    pub choices: Vec<PaneCardChoice>,
    pub reply: Option<PaneCardReply>,
    /// Whether a text terminal can answer it in its own form: choices, a short reply, a typed
    /// secret or a Vault passphrase. A page region, a browser hand-off, a passkey, a payment, a
    /// file pick, an App confirmation or install and a provider sign-in open the pane instead.
    pub text_terminal: bool,
    /// When it was raised; a card id a client has not seen before is new, and a terminal the
    /// person is away from announces it.
    pub requested_at_ms: u64,
    /// The line a terminal shows to answer it elsewhere: the pane's link, or the Room and agent
    /// to open when the kernel has no pairing.
    pub open_elsewhere: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneCardChoice {
    pub id: String,
    pub label: String,
}

/// The typed reply a card takes, answered as `custom_reply` with its choice id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneCardReply {
    pub id: String,
    pub label: String,
    pub placeholder: Option<String>,
    /// Masked as the person types it.
    pub secret: bool,
}

/// One line of work the kernel holds for the agent, in the kernel's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneStripe {
    pub id: String,
    /// What it is: "Queued", "/goal · RUNNING", "Blocked", "/wait-in", "Usage limit" ...
    pub kind: String,
    pub text: String,
    /// When it ends or runs next; a client counts down to it in its own clock.
    pub at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneNotice {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub at_ms: u64,
}
