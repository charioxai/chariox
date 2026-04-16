use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::DaemonError;
use crate::session::PromptAttachment;
use rand::distributions::{Alphanumeric, DistString};

use super::{OpenCodeClient, ProviderResumeState, RuntimeProviderRun};
use crate::provider::opencode_runtime::OpenCodeRuntimeState;

const OPENCODE_EVENT_SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(5);
const OPENCODE_EVENT_SUBSCRIBE_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const OPENCODE_SESSION_CREATE_TIMEOUT: Duration = Duration::from_secs(5);
const OPENCODE_SESSION_CREATE_RETRY_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(crate) struct OpenCodeRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

pub(crate) struct OpenCodeRuntimeBinding {
    pub state: OpenCodeRuntimeState,
    pub selection: OpenCodeRunSelection,
    pub resume_state: ProviderResumeState,
}

pub(crate) fn initialize_opencode_runtime(
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

    let managed_io_permission = run
        .requires_managed_io()
        .then(opencode_managed_io_permission_rules);
    let resumable_session_id = (!run.requires_managed_io())
        .then(|| run.resume_state().opencode_session_id().map(str::to_string))
        .flatten();
    let session_id = match resumable_session_id {
        Some(session_id) if client.snapshot(&session_id).is_ok() => {
            crate::logging::info_with_fields(
                "daemon.provider.opencode",
                "reusing opencode session",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "provider_session_id": session_id.clone(),
                }),
            );
            session_id
        }
        Some(previous_session_id) => {
            crate::logging::warn_with_fields(
                "daemon.provider.opencode",
                "opencode session resume failed; creating a new session",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "provider_session_id": previous_session_id,
                }),
            );
            let session_id = client.create_session_with_retry(
                managed_io_permission.clone(),
                OPENCODE_SESSION_CREATE_TIMEOUT,
                OPENCODE_SESSION_CREATE_RETRY_INTERVAL,
            )?;
            crate::logging::info_with_fields(
                "daemon.provider.opencode",
                "created opencode session",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "provider_session_id": session_id.clone(),
                }),
            );
            session_id
        }
        None => {
            let session_id = client.create_session_with_retry(
                managed_io_permission.clone(),
                OPENCODE_SESSION_CREATE_TIMEOUT,
                OPENCODE_SESSION_CREATE_RETRY_INTERVAL,
            )?;
            crate::logging::info_with_fields(
                "daemon.provider.opencode",
                "created opencode session",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "provider_session_id": session_id.clone(),
                }),
            );
            session_id
        }
    };
    let event_subscription = client.subscribe_events_with_retry(
        OPENCODE_EVENT_SUBSCRIBE_TIMEOUT,
        OPENCODE_EVENT_SUBSCRIBE_RETRY_INTERVAL,
    )?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "subscribed to opencode events",
        serde_json::json!({
            "provider_run_id": run.id(),
        }),
    );

    Ok(OpenCodeRuntimeBinding {
        state: OpenCodeRuntimeState::new(base_url, session_id.clone(), event_subscription),
        selection,
        resume_state: ProviderResumeState::from_opencode_session_id(session_id),
    })
}

fn opencode_managed_io_permission_rules() -> serde_json::Value {
    serde_json::json!([
        {
            "permission": "edit",
            "pattern": "*",
            "action": "deny"
        },
        {
            "permission": "bash",
            "pattern": "*",
            "action": "deny"
        },
        {
            "permission": "task",
            "pattern": "*",
            "action": "deny"
        }
    ])
}

pub(super) fn sync_opencode_run_selection_for_session(
    provider_run_id: &str,
    base_url: &str,
    session_id: &str,
) -> Result<OpenCodeRunSelection, DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, base_url)?;
    let defaults = client.configured_defaults()?;
    let messages = client.messages(session_id)?;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::opencode_managed_io_permission_rules;

    #[test]
    fn managed_io_permission_rules_block_direct_writes() {
        assert_eq!(
            opencode_managed_io_permission_rules(),
            json!([
                {
                    "permission": "edit",
                    "pattern": "*",
                    "action": "deny"
                },
                {
                    "permission": "bash",
                    "pattern": "*",
                    "action": "deny"
                },
                {
                    "permission": "task",
                    "pattern": "*",
                    "action": "deny"
                }
            ])
        );
    }

    #[test]
    fn generated_message_ids_use_opencode_sortable_timestamp_width() {
        let id = super::next_opencode_message_id();

        assert!(id.starts_with("msg_"));
        assert_eq!(id.len(), 30);
        assert!(id[4..16].chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_message_ids_sort_after_current_opencode_ids() {
        let id = super::next_opencode_message_id();

        assert!(
            id.as_str() > "msg_d0000000000000000000000000",
            "generated id {id} should sort in OpenCode's current ascending range"
        );
    }
}

pub(super) fn submit_opencode_prompt(
    run: &RuntimeProviderRun,
    state: &mut OpenCodeRuntimeState,
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(run.id(), state.base_url())?;
    let message_id = next_opencode_message_id();
    client.submit_prompt(
        state.session_id(),
        &message_id,
        prompt,
        attachments,
        Some(run.model()),
        run.variant(),
    )?;
    state.note_prompt_submitted(message_id);
    Ok(())
}

fn next_opencode_message_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed) & 0x0fff;
    let encoded_time =
        timestamp_ms.saturating_mul(0x1000).saturating_add(sequence) & 0xffff_ffff_ffff;
    let random = Alphanumeric.sample_string(&mut rand::thread_rng(), 14);
    format!("msg_{encoded_time:012x}{random}")
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
