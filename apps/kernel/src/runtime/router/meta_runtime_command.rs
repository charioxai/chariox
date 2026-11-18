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
    SetWorkflowNodeWaitForAllInputsRequest, SetWorkflowResponsibleRequest, SliceRefRequest,
    SpawnAgentRequest, UninstallMcpServerRequest, UninstallSkillRequest, UpdateMcpServerRequest,
    UpdateSkillRequest, UpdateWorkflowNodeInstructionsRequest, UpsertCredentialRequest,
    WorkflowResponsibleTarget,
};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::transport::runtime_tools::{MetaRunCommandArgs, RuntimeToolResult};

use super::CommandRouter;

mod agent_peer;
mod agent_tree;
mod agents_run;
mod dispatch;
mod own_schedule;
mod reach_lists;
mod request;
mod result;
mod resume_args;
mod settings_args;
mod spawn_args;
mod summary;
mod workflow_responsible;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod room_admission_tests;
