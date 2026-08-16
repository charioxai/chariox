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

/// Redacts kernel-owned relay credentials before a response crosses any client boundary.
pub(crate) fn redact_client_response_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(serde_json::Value::Object(remote_execution)) =
                object.get_mut("remote_execution")
            {
                remote_execution.remove("relay_token");
            }
            for child in object.values_mut() {
                redact_client_response_value(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                redact_client_response_value(child);
            }
        }
        _ => {}
    }
}
