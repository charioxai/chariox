use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};
use crate::DaemonApp;

pub(super) fn launch_dev_stub_provider(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> RuntimeProviderRun {
    let provider_run = app
        .providers
        .launch_run_detached(
            LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(agent_id),
        )
        .expect("provider run fixture should start");
    app.sessions
        .set_active_provider_run(session_id, Some(provider_run.id().to_string()))
        .expect("provider run fixture should become active");
    app.agents
        .set_agent_runtime_profile_with_account_profile(
            agent_id,
            provider_run.provider(),
            Some(provider_run.model().to_string()),
            provider_run.variant().map(str::to_string),
            Some(provider_run.account_profile().to_string()),
            provider_run.resume_state().clone(),
        )
        .expect("provider run fixture should update its agent profile");
    app.update_provider_run_projection(provider_run.clone());
    provider_run
}

pub(super) fn submit_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    agent_id: &str,
    prompt: &str,
) {
    let prepared = PromptQueueItem::new(
        "pending-draft:projection-test",
        attachment_id,
        agent_id,
        prompt,
        PromptStatus::Queued,
    );
    let outcome = app
        .prompt_owner_submit_prepared_prompt(session_id, prepared, false)
        .expect("prompt fixture should submit");
    if let PromptSubmissionOutcome::Started { prompt } = outcome {
        let provider_run = app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .expect("active provider run fixture should exist");
        app.mark_active_prompt_delivery(
            session_id,
            agent_id,
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run.id().to_string()),
            provider_run.provider_session_id().map(str::to_string),
        )
        .expect("prompt fixture should be delivered");
        crate::transport::flow_control::note_prompt_started(app, provider_run.id());
    }
    crate::app::KernelSessionReadService::new(app)
        .session_snapshot(session_id)
        .expect("prompt fixture projection should refresh");
}

pub(super) fn attach_cli(app: &mut DaemonApp, session_id: &str, client_id: &str) -> String {
    let mut sessions = app.sessions_mut();
    let attachment = app
        .attachments()
        .attach(
            &mut sessions,
            AttachRequest::new(session_id, client_id, ClientCapabilityLevel::FullTerminal),
        )
        .expect("session should attach");
    attachment.id().to_string()
}
