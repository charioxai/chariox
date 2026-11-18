//! A message from another agent that waits for its recipient's next turn, as the person sees it.
use serde::{Deserialize, Serialize};

const ID_PREFIX: &str = "agent-message-";

/// Shown beside the person's own queued prompts: the person can cancel it, or send it now.
/// It is a view of the recipient's inbox row, never stored with the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedAgentMessage {
    /// The queued-prompt id the person's cancel and send controls name.
    pub id: String,
    pub agent_id: String,
    /// The sender as the person knows it: its alias or reference.
    pub sender: String,
    pub prompt: String,
}

impl QueuedAgentMessage {
    pub(crate) fn id_for(sequence: u64) -> String {
        format!("{ID_PREFIX}{sequence}")
    }

    /// The inbox sequence a queued-prompt id names, when it names a queued agent message.
    pub(crate) fn sequence_of(id: &str) -> Option<u64> {
        id.strip_prefix(ID_PREFIX)?.parse().ok()
    }
}
