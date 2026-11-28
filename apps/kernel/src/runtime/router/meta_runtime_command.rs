use crate::agent::GitWorktreePlacement;
use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasAgentRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, AttachToSessionRequest,
    CancelWorkflowRunRequest, CreateSliceRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, DeleteSessionRequest, DestroyAgentRequest, ExtensionKind,
    FocusAgentRequest, GetCredentialRequest, GetCredentialVaultStatusRequest, GetMcpServerRequest,
    GetWorkflowRunRequest, GrantAgentExtensionRequest, ImportMcpServersRequest,
    ImportProviderCapabilitiesRequest, InstallMcpServerRequest, InvokeWorkflowEndpointRequest,
    ListAgentsRequest, ListCredentialsRequest, ListMcpServersRequest, ListWorkflowRunsRequest,
    ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse, ManageCredentialVaultRequest,
    RemoveCredentialRequest, RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest,
    ResolveWorkflowRequest, ResumeWorkflowRunRequest, RevokeAgentExtensionRequest,
    SetWorkflowNodeCanCompleteRunRequest, SetWorkflowNodeCanEmitIntermediateOutputRequest,
    SetWorkflowNodeMaxTurnsRequest, SetWorkflowNodeWaitForAllInputsRequest,
    SetWorkflowResponsibleRequest, SliceRefRequest, SpawnAgentRequest, UninstallMcpServerRequest,
    UpdateMcpServerRequest, UpdateWorkflowNodeInstructionsRequest, UpsertCredentialRequest,
    WorkflowResponsibleTarget,
};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::transport::runtime_tools::{MetaRunCommandArgs, RuntimeToolResult};

use super::CommandRouter;

mod agent_peer;
mod agent_tree;
mod agents_run;
mod dispatch;
mod kernel_identity;
mod own_schedule;
mod provider_accounts;
mod reach_lists;
mod request;
mod result;
mod resume_args;
mod room_lifecycle;
mod settings_args;
mod spawn_args;
mod summary;
mod workflow_responsible;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod room_admission_tests;
