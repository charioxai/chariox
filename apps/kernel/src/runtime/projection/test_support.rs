use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
use crate::DaemonApp;

pub(super) fn launch_dev_stub_provider(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> RuntimeProviderRun {
    let provider_run = app
        .launch_provider(
            LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(agent_id),
        )
        .expect("provider run should launch");
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
    crate::app::KernelAgentService::new(app)
        .submit_prompt(
            session_id,
            attachment_id,
            Some(agent_id),
            prompt,
            Vec::new(),
        )
        .expect("prompt should submit");
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
