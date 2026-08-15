use std::collections::BTreeSet;

use super::{ProviderOutputPump, ProviderOutputPumpRequest};
use crate::app::DaemonApp;
use crate::provider::{ProviderRunState, RuntimeProviderRun};

pub(super) fn pump_session_active_prompt_outputs(
    app: &mut DaemonApp,
    session_id: &str,
) -> Vec<String> {
    let Ok(session) = app.sessions.get_session(session_id) else {
        return Vec::new();
    };
    let recipient_attachment_ids = app.attachments.list_session_attachment_ids(session.id());
    let mut provider_run_ids = BTreeSet::new();
    if let Some(provider_run_id) = session.active_provider_run_id().filter(|run_id| {
        app.providers
            .get_run(run_id)
            .is_ok_and(|run| provider_run_requires_background_pump(app, &session, &run))
    }) {
        provider_run_ids.insert(provider_run_id.to_string());
    }
    for agent_id in app.prompt_state_owner.active_prompt_agent_ids(&session) {
        if let Some(provider_run_id) = app
            .providers
            .get_run_for_agent(session.id(), &agent_id)
            .filter(|run| provider_run_requires_background_pump(app, &session, run))
            .map(|run| run.id().to_string())
        {
            provider_run_ids.insert(provider_run_id);
        }
    }
    provider_run_ids.extend(
        app.providers
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == session.id())
            .filter(should_pump_background_provider_run)
            .map(|run| run.id().to_string()),
    );
    // Keep the final set defensive: every discovery path above must obey the
    // single-owner rule before any legacy pump is invoked.
    provider_run_ids.retain(|provider_run_id| {
        app.providers
            .get_run(provider_run_id)
            .is_ok_and(|run| !crate::provider::provider_run_uses_structured_prompt_io(&run))
    });
    let mut pumped_provider_run_ids = Vec::new();
    for provider_run_id in provider_run_ids {
        let agent_id = app
            .providers
            .get_run(&provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        pumped_provider_run_ids.push(provider_run_id.clone());
        if let Err(error) =
            ProviderOutputPump::new(app).pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: &provider_run_id,
                recipient_attachment_ids: recipient_attachment_ids.clone(),
                initial_liveness_already_checked: false,
            })
        {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "background prompt pump failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
    pumped_provider_run_ids
}

fn should_pump_background_provider_run(run: &RuntimeProviderRun) -> bool {
    !crate::provider::provider_run_uses_structured_prompt_io(run)
        && crate::provider::provider_run_uses_claude_native_bridge(run)
        && matches!(
            run.state(),
            ProviderRunState::Starting | ProviderRunState::Running
        )
}

fn provider_run_requires_background_pump(
    app: &DaemonApp,
    session: &crate::session::RuntimeSession,
    run: &RuntimeProviderRun,
) -> bool {
    // Structured provider output is owned by KernelRuntimeState's single
    // transport-runtime pump. Letting this legacy app/transport path poll the
    // same provider socket races notification consumption and can strand the
    // provider turn without its terminal completion transition.
    if crate::provider::provider_run_uses_structured_prompt_io(run) {
        return false;
    }
    if run.state() == ProviderRunState::Starting || should_pump_background_provider_run(run) {
        return true;
    }
    run.agent_instance_id().is_some_and(|agent_id| {
        app.prompt_state_owner
            .active_prompt_for_agent_snapshot(session, agent_id)
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::should_pump_background_provider_run;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };

    fn structured_run(adapter_key: &str, provider: &str) -> RuntimeProviderRun {
        let request = LaunchProviderRequest::new(
            "session-background-pump-test",
            adapter_key,
            provider,
            "default",
            "test-model",
        );
        let mut run = RuntimeProviderRun::new(
            format!("provider-run-background-{adapter_key}-{provider}"),
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
                structured_endpoint: Some("ws://structured-test".to_string()),
            },
        );
        run.mark_running();
        run
    }

    #[test]
    fn legacy_background_pump_excludes_every_structured_provider() {
        for (adapter_key, provider) in [
            ("codex", "codex"),
            ("opencode", "opencode"),
            ("claude", "claude"),
            ("dev-stub", "slow-structured"),
        ] {
            assert!(
                !should_pump_background_provider_run(&structured_run(adapter_key, provider)),
                "{adapter_key}/{provider} must be owned by the runtime structured pump",
            );
        }
    }
}
