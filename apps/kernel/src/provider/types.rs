use std::fmt;

use serde::{Deserialize, Serialize};

pub(crate) fn provider_workspace_live_sync_mode_by_default(
    provider: &str,
    config: &crate::config::DaemonConfig,
) -> crate::config::WorkspaceLiveSyncMode {
    config.provider_workspace_live_sync_mode(provider)
}

pub(crate) fn provider_workspace_live_sync_mode_for_session(
    provider: &str,
    config: &crate::config::DaemonConfig,
    session: Option<&crate::session::RuntimeSession>,
) -> crate::config::WorkspaceLiveSyncMode {
    session
        .and_then(crate::session::RuntimeSession::workspace_live_sync_mode)
        .unwrap_or_else(|| provider_workspace_live_sync_mode_by_default(provider, config))
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOperation {
    InterruptTurn,
    CancelPrompt,
    AckWorkflowTurn,
    ValidateWorkflowHandoff,
    AttachFile,
    RequestMemoryUpdate,
    RequestCompactionSummary,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlCapabilityMode {
    Native,
    Mcp,
    AdapterEmulated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlCapability {
    operation: ControlOperation,
    mode: ControlCapabilityMode,
}

impl ControlCapability {
    pub fn new(operation: ControlOperation, mode: ControlCapabilityMode) -> Self {
        Self { operation, mode }
    }

    pub fn operation(&self) -> ControlOperation {
        self.operation
    }

    pub fn mode(&self) -> ControlCapabilityMode {
        self.mode
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunState {
    Starting,
    Running,
    Parked,
    Ended,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentEndpointMode {
    Managed,
    External,
}

impl fmt::Display for AgentEndpointMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Managed => "managed",
            Self::External => "external",
        };

        write!(f, "{value}")
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderClientInterface {
    #[default]
    Arroba,
    NativeTui,
}

impl ProviderClientInterface {
    pub fn is_arroba(&self) -> bool {
        matches!(self, Self::Arroba)
    }
}

impl fmt::Display for ProviderClientInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Arroba => "arroba",
            Self::NativeTui => "native_tui",
        };
        write!(f, "{value}")
    }
}

impl fmt::Display for ProviderRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Parked => "parked",
            Self::Ended => "ended",
        };

        write!(f, "{value}")
    }
}
