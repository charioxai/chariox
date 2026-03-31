use std::time::Duration;

use crate::error::DaemonError;
use crate::session::PromptAttachment;

use super::{OpenCodeClient, RuntimeProviderRun};
use crate::provider::opencode_runtime::OpenCodeRuntimeState;

#[derive(Debug, Default)]
pub(super) struct OpenCodeRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

pub(super) struct OpenCodeRuntimeBinding {
    pub state: OpenCodeRuntimeState,
    pub selection: OpenCodeRunSelection,
}

pub(super) fn initialize_opencode_runtime(
    run: &RuntimeProviderRun,
) -> Result<OpenCodeRuntimeBinding, DaemonError> {
    let base_url = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_endpoint_missing",
            message: "opencode run did not expose a structured endpoint".to_string(),
        })?
        .to_string();
    let client = OpenCodeClient::new(run.id(), &base_url)?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "waiting for opencode health",
        serde_json::json!({
            "provider_run_id": run.id(),
            "base_url": base_url.clone(),
        }),
    );
    client.wait_until_healthy(Duration::from_secs(30))?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "opencode became healthy",
        serde_json::json!({
            "provider_run_id": run.id(),
            "base_url": base_url.clone(),
        }),
    );

    let selection = resolve_initial_selection(run, &client)?;

    let session_id = client.create_session()?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "created opencode session",
        serde_json::json!({
            "provider_run_id": run.id(),
            "provider_session_id": session_id.clone(),
        }),
    );
    let event_subscription = client.subscribe_events()?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "subscribed to opencode events",
        serde_json::json!({
            "provider_run_id": run.id(),
        }),
    );

    Ok(OpenCodeRuntimeBinding {
        state: OpenCodeRuntimeState::new(base_url, session_id, event_subscription),
        selection,
    })
}

pub(super) fn runtime_is_healthy(provider_run_id: &str, state: &OpenCodeRuntimeState) -> bool {
    OpenCodeClient::new(provider_run_id, state.base_url())
        .and_then(|client| client.check_health())
        .is_ok()
}

pub(super) fn sync_opencode_run_selection(
    provider_run_id: &str,
    state: &OpenCodeRuntimeState,
) -> Result<OpenCodeRunSelection, DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, state.base_url())?;
    let defaults = client.configured_defaults()?;
    let messages = client.messages(state.session_id())?;

    Ok(OpenCodeRunSelection {
        model: messages
            .iter()
            .rev()
            .find_map(|message| message.info.resolved_model())
            .or(defaults.model),
        variant: messages
            .iter()
            .rev()
            .find_map(|message| message.info.resolved_variant())
            .or(defaults.variant),
    })
}

pub(super) fn abort_opencode_session(
    provider_run_id: &str,
    state: &OpenCodeRuntimeState,
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, state.base_url())?;
    client.abort_session(state.session_id())?;
    Ok(())
}

pub(super) fn submit_opencode_prompt(
    run: &RuntimeProviderRun,
    state: &OpenCodeRuntimeState,
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(run.id(), state.base_url())?;
    client.submit_prompt(
        state.session_id(),
        prompt,
        attachments,
        Some(run.model()),
        run.variant(),
    )?;
    Ok(())
}

fn resolve_initial_selection(
    run: &RuntimeProviderRun,
    client: &OpenCodeClient,
) -> Result<OpenCodeRunSelection, DaemonError> {
    if run.model() != "default" && run.variant().is_some() {
        crate::logging::debug_with_fields(
            "daemon.provider.opencode",
            "skipped configured defaults lookup for explicit model and variant",
            serde_json::json!({
                "provider_run_id": run.id(),
                "requested_model": run.model(),
                "requested_variant": run.variant(),
            }),
        );
        return Ok(OpenCodeRunSelection::default());
    }

    let resolved = client.configured_defaults()?;
    crate::logging::debug_with_fields(
        "daemon.provider.opencode",
        "checked opencode configured defaults",
        serde_json::json!({
            "provider_run_id": run.id(),
            "requested_model": run.model(),
            "requested_variant": run.variant(),
            "selected_agent": resolved.selected_agent,
            "agent_model": resolved.agent_model,
            "agent_variant": resolved.agent_variant,
            "top_level_model": resolved.top_level_model,
            "resolved_model": resolved.model,
            "resolved_variant": resolved.variant,
        }),
    );

    Ok(OpenCodeRunSelection {
        model: (run.model() == "default")
            .then_some(resolved.model)
            .flatten(),
        variant: run
            .variant()
            .is_none()
            .then_some(resolved.variant)
            .flatten(),
    })
}
