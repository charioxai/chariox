use super::*;

use crate::room_terminal::{
    RoomTerminalInputResult, RoomTerminalOutput, RoomTerminalSnapshot, RoomTerminalSummary,
    RoomTerminalWaitResult, RoomTerminalWatch,
};

/// One operation on a Room's terminals. Only the Room's owner may send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTerminalRequest {
    pub session_id: String,
    /// Speak for this Room agent instead of the signed-in user. The runtime
    /// tool bridge sets it for agents; the one-driver rule treats each agent
    /// separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acting_agent: Option<String>,
    /// Run on the machine where this Room agent runs (a leased worker or a
    /// slice). Defaults to the machine of `acting_agent`, else this kernel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_agent: Option<String>,
    pub operation: RoomTerminalOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum RoomTerminalOperation {
    /// A login shell in the Room's project folder, or `command` (argv, no shell).
    Open {
        cols: u16,
        rows: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<Vec<String>>,
    },
    List,
    Snapshot {
        terminal_id: String,
    },
    /// Keystrokes as one unit; refused while another actor drives a program.
    Input {
        terminal_id: String,
        data_base64: String,
    },
    Resize {
        terminal_id: String,
        cols: u16,
        rows: u16,
    },
    /// Output after `after_offset`, waiting up to `wait_ms` for some.
    ReadOutput {
        terminal_id: String,
        after_offset: u64,
        #[serde(default)]
        wait_ms: u64,
    },
    /// Block until a condition holds, capped below the provider tool limit.
    Wait {
        terminal_id: String,
        watches: Vec<RoomTerminalWatch>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_offset: Option<u64>,
        timeout_ms: u64,
    },
    Close {
        terminal_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RoomTerminalResult {
    Opened { terminal: RoomTerminalSummary },
    Listed { terminals: Vec<RoomTerminalSummary> },
    Snapshot { snapshot: RoomTerminalSnapshot },
    Input { input: RoomTerminalInputResult },
    Resized { terminal: RoomTerminalSummary },
    Output { output: RoomTerminalOutput },
    Waited { waited: RoomTerminalWaitResult },
    Closed,
}

impl RoomTerminalResult {
    /// Mark the terminals in a worker's answer with the agent whose machine
    /// runs them, so a client can address them again.
    pub(crate) fn hosted_by(mut self, agent_id: &str) -> Self {
        let host =
            |terminal: &mut RoomTerminalSummary| terminal.host_agent = Some(agent_id.to_string());
        match &mut self {
            Self::Opened { terminal } | Self::Resized { terminal } => host(terminal),
            Self::Listed { terminals } => terminals.iter_mut().for_each(host),
            Self::Snapshot { snapshot } => host(&mut snapshot.terminal),
            Self::Output { output } => host(&mut output.terminal),
            Self::Waited { waited } => host(&mut waited.terminal),
            Self::Input { .. } | Self::Closed => {}
        }
        self
    }
}
