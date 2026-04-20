use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;
use crate::attachment::{ClientCapabilityLevel, RuntimeAttachment};
use crate::capability::{
    CaptureScreenshotResult, EditFileResult, InspectGitResult, ReadDirectoryTreeResult,
    ReadFileResult, RunShellCommandResult, StoredTransferArtifact,
};
use crate::config::ArrobaUserConfig;
use crate::history::HistoryEvent;
use crate::mcp::{ArrobaMcpServerConfig, McpImportOutcome};
use crate::provider::{
    OpenCodeProviderCatalog, ProviderAuthStatus, ProviderCommandCatalog, ProviderLoginStart,
    ProviderProcessInfo, RuntimeProviderRun,
};
use crate::runtime::projection::DaemonHealthProjection;
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion,
    PromptSubmissionOutcome, QueuedWorkflowLaunch, RuntimeSession, SessionConfigState,
    SessionInvite, SessionMember, WorkflowDefinition, WorkflowEdgeDefinition,
    WorkflowEndpointDefinition, WorkflowLaunchPolicy, WorkflowNodeDefinition, WorkflowRun,
    WorkflowWatchdogDefinition, WorkflowWatchdogPolicy, WorkspaceLinkAttachment,
    WorkspaceLinkDefinition,
};
use crate::session_history_page::{SessionHistoryCursor, SessionHistoryPageEntry};
use crate::skill::{ArrobaSkillMetadata, SkillImportOutcome};
#[cfg(test)]
mod tests;
mod types;

pub use types::*;
