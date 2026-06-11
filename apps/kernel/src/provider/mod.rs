mod claude;
mod claude_runtime;
mod codex;
mod codex_client;
mod codex_runtime;
mod command_catalog;
mod launch_contract;
mod mcp_proxy;
mod opencode;
mod opencode_binding;
mod opencode_client;
mod opencode_runtime;
mod process_info;
mod prompt_signals;
mod registry;
mod run_actor;
mod runtime_run;
mod service;
mod types;
mod workspace_live_sync_policy;
mod workspace_write_fence;

pub use claude::{claude_provider_catalog, plan_claude_launch, resolve_claude_executable};
pub(crate) use claude_runtime::ClaudeRuntimeState;
pub use codex::{
    codex_catalog_endpoint, ensure_codex_catalog_endpoint, logout_codex, plan_codex_launch,
    resolve_codex_executable,
};
pub use codex_client::{
    CodexClient, CodexNotification, CodexRunSelection, CodexSocket, CodexThread,
    CodexThreadStartResponse, ProviderAuthStatus, ProviderLoginStart,
};
pub use codex_runtime::{run_codex_utility_prompt, CodexRuntimeState};
pub use command_catalog::{
    default_provider_command_catalogs, ProviderCommandCatalog, ProviderCommandCatalogDiscovery,
    ProviderCommandCatalogSource, ProviderCommandDescriptor,
};
pub use launch_contract::{
    AgentExecutionMode, AgentPermissionLevel, ExternalProviderImportMetadata,
    ExternalProviderImportMode, ExternalProviderObservedCursor, LaunchProviderRequest,
    ProviderLaunchResult, ProviderResumeState, ProviderWriteAccessMode, RuntimeMcpBinding,
};
pub(crate) use mcp_proxy::dispatch_provider_mcp_proxy_request;
pub use opencode::{
    ensure_opencode_catalog_endpoint, opencode_catalog_endpoint, plan_opencode_launch,
    resolve_opencode_executable,
};
pub(crate) use opencode_binding::run_opencode_utility_prompt;
pub use opencode_client::{
    OpenCodeClient, OpenCodeEvent, OpenCodeEventSubscription, OpenCodeMessage,
    OpenCodeMessageCacheTokens, OpenCodeMessageInfo, OpenCodeMessageTime, OpenCodeMessageTokens,
    OpenCodePart, OpenCodePartTime, OpenCodeProviderCatalog, OpenCodeProviderInfo,
    OpenCodeProviderModel, OpenCodeProviderModelLimit, OpenCodeSelectedModel,
    OpenCodeSessionSnapshot, OpenCodeToolState,
};
pub use process_info::{ProviderProcessInfo, ProviderProcessStatus};
pub(crate) use prompt_signals::{
    classify_provider_substitutable_failure_text, classify_provider_terminal_failure_text,
};
pub use prompt_signals::{
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
};
pub use registry::{AgentEndpointAdapter, ProviderRegistry};
pub(crate) use run_actor::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution, ProviderRunActorMailbox,
    ProviderRunOperationLanes,
};
pub use runtime_run::{ProviderRunTokenUsage, RuntimeProviderRun};
pub use service::{ProviderProcessService, ProviderProcessServiceStore};
pub(crate) use service::{ProviderRunLivenessReconciliation, ProviderRuntimeBinding};
pub(crate) use types::provider_workspace_live_sync_mode_for_session;
pub use types::{
    AgentEndpointMode, ControlCapability, ControlCapabilityMode, ControlOperation,
    ProviderClientInterface, ProviderRunState,
};
pub(crate) use workspace_live_sync_policy::{
    native_tui_hidden_instructions_block, NATIVE_TUI_HIDDEN_INSTRUCTIONS_END,
    NATIVE_TUI_HIDDEN_INSTRUCTIONS_START, WORKSPACE_LIVE_SYNC_INSTRUCTIONS_SOURCE_PATH,
};

pub(crate) fn adapter_key_for_provider(provider: &str) -> &str {
    match provider {
        "default" => "opencode",
        "claude-headless" | "claude-p" => "claude",
        value => value,
    }
}
pub(crate) use workspace_write_fence::{
    apply_workspace_write_fence, workspace_write_fence_active, workspace_write_fence_backend,
    workspace_write_fence_supported, workspace_write_fence_unavailable_reason,
};
