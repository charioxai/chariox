//! Wire and domain shapes of Room terminals (protocol 561).

use serde::{Deserialize, Serialize};

/// Who types into a terminal: the signed-in user, or one Room agent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "agent_id", rename_all = "snake_case")]
pub enum TerminalActor {
    User,
    Agent(String),
}

impl std::fmt::Display for TerminalActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => formatter.write_str("user"),
            Self::Agent(agent_id) => formatter.write_str(agent_id),
        }
    }
}

/// The one-driver state every client and agent sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RoomTerminalDriver {
    /// The shell is at its prompt: any Room agent can type.
    Free,
    /// A foreground program belongs to `driven_by` until it exits.
    Busy { driven_by: TerminalActor },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum RoomTerminalOutcome {
    ShellExited {
        exit_code: Option<u32>,
        signal: Option<String>,
    },
    KernelRestarted,
}

impl RoomTerminalOutcome {
    pub fn description(&self) -> String {
        match self {
            Self::KernelRestarted => "ended: kernel restarted".to_string(),
            Self::ShellExited {
                exit_code: Some(code),
                ..
            } => format!("ended: shell exited with code {code}"),
            Self::ShellExited {
                signal: Some(signal),
                ..
            } => format!("ended: shell stopped by {signal}"),
            Self::ShellExited { .. } => "ended: shell exited".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RoomTerminalStatus {
    Running,
    Ended {
        outcome: RoomTerminalOutcome,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTerminalSummary {
    pub terminal_id: String,
    pub session_id: String,
    pub title: String,
    pub program: String,
    pub working_directory: String,
    pub cols: u16,
    pub rows: u16,
    pub status: RoomTerminalStatus,
    pub driver: RoomTerminalDriver,
    pub opened_by: TerminalActor,
    pub created_at_ms: u64,
    /// Bytes of output produced so far: the cursor for `ReadRoomTerminalOutput`.
    pub output_offset: u64,
}

/// The screen as a person sees it plus the lines that scrolled off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTerminalSnapshot {
    pub terminal: RoomTerminalSummary,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub screen_text: String,
    pub scrollback_text: String,
    /// Base64 of the escape sequences that redraw the screen in a terminal emulator.
    pub screen_ansi_base64: String,
    /// Output continues from this offset with no gap.
    pub output_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTerminalOutput {
    pub terminal: RoomTerminalSummary,
    pub data_base64: String,
    pub next_offset: u64,
    /// The requested offset is no longer retained: read a snapshot to resync.
    pub gap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTerminalInputResult {
    pub accepted: bool,
    pub driver: RoomTerminalDriver,
    /// Why the input was refused, e.g. `busy (driven by agent-a)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

/// What an agent can ask to be woken for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomTerminalWatch {
    /// The shell exits, or the foreground program returns to the prompt.
    Finished,
    /// New output contains this text.
    OutputContains(String),
    /// A foreground program has been quiet for a few seconds.
    WaitingForInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomTerminalEventKind {
    Finished {
        /// Known when the shell itself exited; a foreground program that
        /// returned to the prompt leaves its status to the screen.
        exit_code: Option<u32>,
        signal: Option<String>,
    },
    OutputMatched {
        text: String,
    },
    WaitingForInput,
}

/// A fired watch with the screen the agent needs to continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomTerminalEvent {
    pub terminal_id: String,
    pub session_id: String,
    pub armed_by: String,
    pub kind: RoomTerminalEventKind,
    pub screen_text: String,
}
