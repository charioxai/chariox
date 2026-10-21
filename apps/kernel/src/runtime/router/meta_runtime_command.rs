use crate::agent::GitWorktreePlacement;
use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasAgentRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, AttachToSessionRequest,
    CancelWorkflowRunRequest, CreateSliceRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, DestroyAgentRequest, ExtensionKind, FocusAgentRequest,
    GetCredentialRequest, GetCredentialVaultStatusRequest, GetMcpServerRequest,
    GetWorkflowRunRequest, GrantAgentExtensionRequest, ImportMcpServersRequest,
    ImportProviderCapabilitiesRequest, InstallMcpServerRequest, InvokeWorkflowEndpointRequest,
    ListAgentsRequest, ListCredentialsRequest, ListMcpServersRequest, ListWorkflowRunsRequest,
    ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse, ManageCredentialVaultRequest,
    RemoveCredentialRequest, RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest,
    ResolveWorkflowRequest, ResumeWorkflowRunRequest, RevokeAgentExtensionRequest,
    SetWorkflowNodeCanCompleteRunRequest, SetWorkflowNodeCanEmitIntermediateOutputRequest,
    SetWorkflowNodeMaxTurnsRequest, SetWorkflowNodeWaitForAllInputsRequest, SliceRefRequest,
    SpawnAgentRequest, UninstallMcpServerRequest, UpdateMcpServerRequest,
    UpdateWorkflowNodeInstructionsRequest, UpsertCredentialRequest,
};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::transport::runtime_tools::{MetaRunCommandArgs, RuntimeToolResult};

use super::CommandRouter;

mod dispatch;
mod request;
mod result;
mod spawn_args;
mod summary;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod room_admission_tests;
