use crate::agent::GitWorktreePlacement;
use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasAgentRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, AttachToSessionRequest,
    CancelWorkflowRunRequest, CreateSliceRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, DestroyAgentRequest, ExtensionKind, FocusAgentRequest,
    GetCredentialRequest, GetCredentialVaultStatusRequest, GetMcpServerRequest, GetSkillRequest,
    GetWorkflowRunRequest, GrantAgentExtensionRequest, ImportMcpServersRequest,
    ImportProviderCapabilitiesRequest, ImportSkillsRequest, InstallMcpServerRequest,
    InstallSkillRequest, InvokeWorkflowEndpointRequest, ListAgentsRequest, ListCredentialsRequest,
    ListMcpServersRequest, ListSkillsRequest, ListWorkflowRunsRequest, ListWorkflowsRequest,
    LocalDaemonRequest, LocalDaemonResponse, ManageCredentialVaultRequest, RemoveCredentialRequest,
    RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest, ResolveWorkflowRequest,
    ResumeWorkflowRunRequest, RevokeAgentExtensionRequest, SetWorkflowNodeCanCompleteRunRequest,
    SetWorkflowNodeCanEmitIntermediateOutputRequest, SetWorkflowNodeMaxTurnsRequest,
    SetWorkflowNodeWaitForAllInputsRequest, SliceRefRequest, SpawnAgentRequest,
    UninstallMcpServerRequest, UninstallSkillRequest, UpdateMcpServerRequest, UpdateSkillRequest,
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
