use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;
use crate::attachment::{ClientCapabilityLevel, RuntimeAttachment};
use crate::capability::{
    CaptureScreenshotResult, EditFileResult, InspectGitResult, ReadDirectoryTreeResult,
    ReadFileResult, RunShellCommandResult, StoredTransferArtifact,
};
use crate::config::{CharioxUserConfig, UserCredentialConfig};
use crate::connector::{
    CharioxConnectorAdapterDefinition, CharioxConnectorDefinition, ConnectorExecution,
};
use crate::history::{HistoryEvent, SessionHistoryEntryKind};
use crate::mcp::{CharioxMcpServerConfig, McpImportOutcome};
use crate::provider::{
    OpenCodeProviderCatalog, ProviderAuthStatus, ProviderCommandCatalog, ProviderLoginStart,
    ProviderProcessInfo, RuntimeProviderRun,
};
use crate::runtime::projection::DaemonHealthProjection;
use crate::script::{CharioxEnvironmentConfig, CharioxScriptMetadata};
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion,
    PromptSubmissionOutcome, RuntimeSession, SessionConfigState, SessionInvite, SessionMember,
    WorkflowCanvasLayout, WorkflowCanvasLayoutPatch, WorkflowDefinition, WorkflowEdgeDefinition,
    WorkflowEndpointDefinition, WorkflowNodeDefinition, WorkflowPromptQueueDefinition,
    WorkflowPublicationDefinition, WorkflowQueuedPrompt, WorkflowRun, WorkflowScheduleDefinition,
    WorkflowScheduleOverlapPolicy, WorkflowScheduleTrigger, WorkflowWatchdogDefinition,
    WorkflowWatchdogPolicy, WorkspaceLinkAttachment, WorkspaceLinkDefinition,
};
pub use crate::session::{WorkflowPublicationSnapshot, WorkflowPublicationSourceSessionSnapshot};
use crate::session_history_page::SessionHistoryPageEntry;
use crate::skill::{CharioxSkillMetadata, SkillImportOutcome};
#[cfg(test)]
mod tests;
mod types;

pub use types::*;
