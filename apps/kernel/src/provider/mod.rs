use std::time::Duration;

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
pub use codex_runtime::CodexRuntimeState;
pub use command_catalog::{
    default_provider_command_catalogs, ProviderCommandCatalog, ProviderCommandCatalogDiscovery,
    ProviderCommandCatalogSource, ProviderCommandDescriptor,
};
pub use launch_contract::{
    canonical_external_provider_session_id, default_provider_control_capabilities,
    external_provider_import_model, external_provider_session_providers,
    normalize_provider_resume_model, provider_resume_failure_notice,
    provider_uses_inferred_runtime_mcp_binding, AgentExecutionMode, AgentPermissionLevel,
    ExternalProviderImportMetadata, ExternalProviderObservedCursor, LaunchProviderRequest,
    ProviderLaunchResult, ProviderResumeState, ProviderWriteAccessMode, RuntimeMcpBinding,
};
pub(crate) use mcp_proxy::dispatch_provider_mcp_proxy_request;
pub use opencode::{
    ensure_opencode_catalog_endpoint, opencode_catalog_endpoint, plan_opencode_launch,
    resolve_opencode_executable,
};
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
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
    ProviderRunActorCompletionSignal, ProviderRunActorMailbox, ProviderRunOperationLanes,
};
pub(crate) use runtime_run::projected_leased_provider_run_id;
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

pub(crate) fn provider_id_for_launch(provider: &str) -> &str {
    match provider {
        "default" => "opencode",
        value => value,
    }
}

pub(crate) fn provider_run_is_claude_headless(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "claude" && run.provider() == "claude-headless"
}

pub(crate) fn provider_run_uses_claude_native_bridge(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "claude"
        && (!run.client_interface().is_arroba() || provider_run_is_claude_headless(run))
}

pub(crate) fn provider_run_uses_structured_prompt_io(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "codex"
        || (run.adapter_key() == "claude"
            && run.client_interface().is_arroba()
            && !provider_run_uses_claude_native_bridge(run))
        || run.adapter_key() == "opencode"
        || (run.adapter_key() == "dev-stub" && run.provider() == "slow-structured")
}

pub(crate) fn provider_run_finalizes_cancellation_on_abort_dispatch(
    run: &RuntimeProviderRun,
) -> bool {
    run.adapter_key() == "claude" && provider_run_uses_structured_prompt_io(run)
}

pub(crate) fn provider_run_supports_selection_sync(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "opencode"
}

pub(crate) fn provider_run_refreshes_selection_on_read(run: &RuntimeProviderRun) -> bool {
    provider_run_supports_selection_sync(run) && run.client_interface().is_arroba()
}

pub(crate) fn provider_run_waits_for_workflow_publication_completion(
    run: &RuntimeProviderRun,
) -> bool {
    run.adapter_key() == "claude" && provider_run_uses_structured_prompt_io(run)
}

pub(crate) fn provider_run_reuses_run_for_mcp_continuation_reload(
    run: &RuntimeProviderRun,
) -> bool {
    run.adapter_key() == "opencode"
}

pub(crate) fn provider_adapter_supports_policy_reload(adapter_key: &str) -> bool {
    matches!(adapter_key, "claude" | "codex" | "opencode")
}

pub(crate) fn provider_batch_launch_concurrency_limit(
    adapter_key: &str,
    provider: &str,
    default_limit: usize,
) -> usize {
    if adapter_key == "dev-stub" || provider == "dev-stub" {
        return 64;
    }
    if matches!(adapter_key, "codex" | "opencode" | "claude" | "claude-code")
        || matches!(provider, "codex" | "opencode" | "claude" | "claude-code")
    {
        return 16;
    }
    default_limit
}

pub(crate) fn provider_run_supports_policy_reload(run: &RuntimeProviderRun) -> bool {
    provider_adapter_supports_policy_reload(run.adapter_key())
}

pub(crate) fn provider_run_uses_runtime_structured_utility_prompt(
    run: &RuntimeProviderRun,
) -> bool {
    run.adapter_key() == "claude" && run.client_interface().is_arroba()
}

pub(crate) fn run_blocking_provider_utility_prompt(
    run: &RuntimeProviderRun,
    visible_user_prompt: &str,
    hidden_system_context: &str,
    timeout: Duration,
    operation: &'static str,
) -> Result<String, crate::error::DaemonError> {
    match run.adapter_key() {
        "codex" => codex_runtime::run_codex_utility_prompt(
            run,
            visible_user_prompt,
            hidden_system_context,
            timeout,
        ),
        "opencode" => opencode_binding::run_opencode_utility_prompt(
            run,
            visible_user_prompt,
            hidden_system_context,
            timeout,
        ),
        adapter_key => Err(crate::error::DaemonError::LocalTransport {
            operation,
            message: format!(
                "agent utility prompts are not supported for provider adapter `{adapter_key}`"
            ),
        }),
    }
}
pub(crate) use workspace_write_fence::{
    apply_workspace_write_fence, workspace_write_fence_active, workspace_write_fence_backend,
    workspace_write_fence_supported, workspace_write_fence_unavailable_reason,
};

#[cfg(test)]
mod tests {
    use super::{
        adapter_key_for_provider, provider_adapter_supports_policy_reload,
        provider_batch_launch_concurrency_limit, provider_id_for_launch,
        provider_run_finalizes_cancellation_on_abort_dispatch, provider_run_is_claude_headless,
        provider_run_refreshes_selection_on_read, provider_run_supports_policy_reload,
        provider_run_reuses_run_for_mcp_continuation_reload, provider_run_supports_selection_sync,
        provider_run_uses_claude_native_bridge, provider_run_uses_runtime_structured_utility_prompt,
        provider_run_waits_for_workflow_publication_completion,
        run_blocking_provider_utility_prompt, AgentEndpointMode, LaunchProviderRequest,
        ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
    };

    #[test]
    fn claude_headless_provider_mode_is_provider_policy() {
        let headless = provider_run("claude", "claude-headless");
        let regular = provider_run("claude", "claude");

        assert!(provider_run_is_claude_headless(&headless));
        assert!(provider_run_uses_claude_native_bridge(&headless));
        assert!(!provider_run_is_claude_headless(&regular));
        assert!(!provider_run_uses_claude_native_bridge(&regular));
    }

    #[test]
    fn provider_launch_identity_normalization_is_provider_policy() {
        assert_eq!(adapter_key_for_provider("default"), "opencode");
        assert_eq!(provider_id_for_launch("default"), "opencode");
        assert_eq!(adapter_key_for_provider("claude-headless"), "claude");
        assert_eq!(provider_id_for_launch("claude-headless"), "claude-headless");
        assert_eq!(adapter_key_for_provider("codex"), "codex");
        assert_eq!(provider_id_for_launch("codex"), "codex");
    }

    #[test]
    fn structured_claude_cancellation_settlement_is_provider_policy() {
        let structured = provider_run("claude", "claude");
        let headless = provider_run("claude", "claude-headless");
        let native_tui = provider_run_with_client_interface(
            "claude",
            "claude",
            ProviderClientInterface::NativeTui,
        );
        let codex = provider_run("codex", "codex");

        assert!(provider_run_finalizes_cancellation_on_abort_dispatch(
            &structured
        ));
        assert!(!provider_run_finalizes_cancellation_on_abort_dispatch(
            &headless
        ));
        assert!(!provider_run_finalizes_cancellation_on_abort_dispatch(
            &native_tui
        ));
        assert!(!provider_run_finalizes_cancellation_on_abort_dispatch(
            &codex
        ));
    }

    #[test]
    fn opencode_selection_refresh_is_provider_policy() {
        let arroba_opencode = provider_run("opencode", "opencode");
        let native_opencode = provider_run_with_client_interface(
            "opencode",
            "opencode",
            ProviderClientInterface::NativeTui,
        );
        let codex = provider_run("codex", "codex");

        assert!(provider_run_supports_selection_sync(&arroba_opencode));
        assert!(provider_run_supports_selection_sync(&native_opencode));
        assert!(!provider_run_supports_selection_sync(&codex));

        assert!(provider_run_refreshes_selection_on_read(&arroba_opencode));
        assert!(!provider_run_refreshes_selection_on_read(&native_opencode));
        assert!(!provider_run_refreshes_selection_on_read(&codex));
    }

    #[test]
    fn workflow_publication_completion_wait_is_provider_policy() {
        let structured_claude = provider_run("claude", "claude");
        let headless_claude = provider_run("claude", "claude-headless");
        let codex = provider_run("codex", "codex");
        let opencode = provider_run("opencode", "opencode");

        assert!(provider_run_waits_for_workflow_publication_completion(
            &structured_claude
        ));
        assert!(!provider_run_waits_for_workflow_publication_completion(
            &headless_claude
        ));
        assert!(!provider_run_waits_for_workflow_publication_completion(
            &codex
        ));
        assert!(!provider_run_waits_for_workflow_publication_completion(
            &opencode
        ));
    }

    #[test]
    fn mcp_continuation_reload_run_reuse_is_provider_policy() {
        let opencode = provider_run("opencode", "opencode");
        let codex = provider_run("codex", "codex");
        let claude = provider_run("claude", "claude");

        assert!(provider_run_reuses_run_for_mcp_continuation_reload(
            &opencode
        ));
        assert!(!provider_run_reuses_run_for_mcp_continuation_reload(&codex));
        assert!(!provider_run_reuses_run_for_mcp_continuation_reload(
            &claude
        ));
    }

    #[test]
    fn provider_policy_reload_support_is_provider_policy() {
        for adapter in ["claude", "codex", "opencode"] {
            assert!(
                provider_adapter_supports_policy_reload(adapter),
                "{adapter} should relaunch when launch-time runtime config changes"
            );
            assert!(
                provider_run_supports_policy_reload(&provider_run(adapter, adapter)),
                "{adapter} runs should relaunch when launch-time runtime config changes"
            );
        }
        for adapter in ["dev-stub", "unknown"] {
            assert!(
                !provider_adapter_supports_policy_reload(adapter),
                "{adapter} should not use provider relaunch policy"
            );
            assert!(
                !provider_run_supports_policy_reload(&provider_run(adapter, adapter)),
                "{adapter} runs should not use provider relaunch policy"
            );
        }
    }

    #[test]
    fn provider_batch_launch_concurrency_limit_is_provider_policy() {
        assert_eq!(
            provider_batch_launch_concurrency_limit("codex", "codex", 99),
            16
        );
        assert_eq!(
            provider_batch_launch_concurrency_limit("default-adapter", "opencode", 99),
            16
        );
        assert_eq!(
            provider_batch_launch_concurrency_limit("dev-stub", "codex", 99),
            64
        );
        assert_eq!(
            provider_batch_launch_concurrency_limit("custom", "custom", 99),
            99
        );
    }

    #[test]
    fn runtime_structured_utility_prompt_is_provider_policy() {
        let structured_claude = provider_run("claude", "claude");
        let native_claude = provider_run_with_client_interface(
            "claude",
            "claude",
            ProviderClientInterface::NativeTui,
        );
        let codex = provider_run("codex", "codex");
        let opencode = provider_run("opencode", "opencode");

        assert!(provider_run_uses_runtime_structured_utility_prompt(
            &structured_claude
        ));
        assert!(!provider_run_uses_runtime_structured_utility_prompt(
            &native_claude
        ));
        assert!(!provider_run_uses_runtime_structured_utility_prompt(&codex));
        assert!(!provider_run_uses_runtime_structured_utility_prompt(
            &opencode
        ));
    }

    #[test]
    fn blocking_utility_prompt_reports_unsupported_adapter() {
        let run = provider_run("dev-stub", "utility-unsupported");
        let error = run_blocking_provider_utility_prompt(
            &run,
            "visible",
            "hidden",
            std::time::Duration::from_secs(1),
            "test utility",
        )
        .expect_err("unsupported adapter should fail before provider I/O");

        match error {
            crate::error::DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "test utility");
                assert!(message.contains("dev-stub"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn provider_run(adapter_key: &str, provider: &str) -> RuntimeProviderRun {
        provider_run_with_client_interface(adapter_key, provider, ProviderClientInterface::Arroba)
    }

    fn provider_run_with_client_interface(
        adapter_key: &str,
        provider: &str,
        client_interface: ProviderClientInterface,
    ) -> RuntimeProviderRun {
        let request =
            LaunchProviderRequest::new("session-1", adapter_key, provider, "default", "model")
                .with_client_interface(client_interface);
        RuntimeProviderRun::new(
            format!("provider-run-{adapter_key}-{provider}"),
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: provider.to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }
}
