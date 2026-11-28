use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandTarget {
    Kernel,
    RoomWorkspace {
        session_id: String,
    },
    AgentSlice {
        session_id: String,
        agent_id: String,
    },
}

impl CommandTarget {
    pub(crate) fn session_id(&self) -> Option<&str> {
        match self {
            Self::Kernel => None,
            Self::RoomWorkspace { session_id } | Self::AgentSlice { session_id, .. } => {
                Some(session_id)
            }
        }
    }

    pub(crate) fn host_agent(&self) -> Option<&str> {
        match self {
            Self::AgentSlice { agent_id, .. } => Some(agent_id),
            _ => None,
        }
    }
}

/// Start or operate the pipes of a command on an explicit execution target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCommandRequest {
    pub target: CommandTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acting_agent: Option<String>,
    pub operation: RunCommandOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum RunCommandOperation {
    ExecList,
    ExecOpen {
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directory: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Kernel job launch: its command receives only the explicitly selected project environment.
    ExecOpenJob {
        command: Vec<String>,
        #[serde(default)]
        needs: JobNeeds,
    },
    ExecStartNow {
        exec_id: String,
    },
    /// Run a launch that is still held on another of the owner's kernels. The
    /// target Room is made in `directory` there, else in that kernel's own default Room directory.
    ExecPlace {
        exec_id: String,
        kernel: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directory: Option<String>,
    },
    ExecWrite {
        exec_id: String,
        offset: u64,
        data_base64: String,
    },
    ExecStdinClose {
        exec_id: String,
        offset: u64,
    },
    ExecRead {
        exec_id: String,
        after_cursor: u64,
        #[serde(default)]
        wait_ms: u64,
    },
    ExecCancel {
        exec_id: String,
    },
    ExecClose {
        exec_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RunCommandResult {
    ExecListed {
        execs: Vec<crate::room_exec::ExecSummary>,
    },
    ExecOpened {
        exec_id: String,
    },
    ExecInput {
        next_offset: u64,
    },
    ExecOutput {
        output: crate::room_exec::ExecOutput,
    },
    ExecCancelled,
    ExecClosed,
}
