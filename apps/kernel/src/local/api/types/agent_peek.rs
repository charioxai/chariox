//! Protocol 1006: peek into an agent's transcript, the one the pane renders, a little at a
//! time: its latest entries as titles, then one entry's content by pages.
use super::*;

/// The last `limit` entries of an agent's transcript before `before`, titles only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeekAgentTranscriptRequest {
    pub session_id: String,
    pub agent_id: String,
    /// At most `MAX_AGENT_PEEK_ENTRIES`; `DEFAULT_AGENT_PEEK_ENTRIES` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// The `before` cursor of an earlier page: entries older than it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

/// One entry's content from character `offset`, at most `limit` characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadAgentTranscriptEntryRequest {
    pub session_id: String,
    pub agent_id: String,
    pub entry_id: String,
    #[serde(default)]
    pub offset: usize,
    /// At most `MAX_AGENT_PEEK_PAGE_CHARS`; `DEFAULT_AGENT_PEEK_PAGE_CHARS` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

pub const DEFAULT_AGENT_PEEK_ENTRIES: usize = 10;
pub const MAX_AGENT_PEEK_ENTRIES: usize = 50;
pub const DEFAULT_AGENT_PEEK_PAGE_CHARS: usize = 4_000;
pub const MAX_AGENT_PEEK_PAGE_CHARS: usize = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTranscriptEntryKind {
    Prompt,
    Assistant,
    Thinking,
    ToolCall,
    Wait,
    Error,
    Notice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTranscriptEntryStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

/// An entry as a title row: what the pane shows before the row is opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTranscriptTitle {
    /// Names the transcript sequences the row covers: a running row's id changes as it grows, so
    /// peek again before reading it once it has ended.
    pub entry_id: String,
    pub kind: AgentTranscriptEntryKind,
    pub title: String,
    pub status: AgentTranscriptEntryStatus,
    pub at_ms: u64,
    /// How long it took, or for a running entry how long it has been running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The size of its content, for `ReadAgentTranscriptEntry` pages.
    pub chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTranscriptPeek {
    pub agent_id: String,
    /// Oldest first; the last one is the agent's latest entry.
    pub entries: Vec<AgentTranscriptTitle>,
    /// Pass back as `before` for the entries older than these; absent at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTranscriptEntryPage {
    pub entry_id: String,
    pub kind: AgentTranscriptEntryKind,
    pub title: String,
    pub offset: usize,
    pub text: String,
    pub total_chars: usize,
    /// Pass back as `offset` for the rest; absent once the page reaches the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}
